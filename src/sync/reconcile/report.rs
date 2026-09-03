//! The [`ReconcileReport`] returned by a reconciliation pass.

/// Report of a reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ReconcileReport {
    /// Files that were re-indexed because their hash differed from the
    /// stored hash or because they were new to the database.
    pub reconciled: usize,
    /// Files whose hash matched the stored hash and were skipped.
    pub unchanged: usize,
    /// Files that were removed from the database.
    pub deleted: usize,
    /// Dirty paths the project's ignore rules excluded — not indexed, the
    /// same way a fresh publish would skip them.
    pub ignored: usize,
    /// Dirty paths skipped because they exceed `limits.max_file_bytes` —
    /// recorded so the report stays honest about what was not indexed and
    /// why, mirroring the full-publish path's per-file skips (the file
    /// cannot be indexed at all, and it must not fail the whole reconcile).
    pub skipped: usize,
}
