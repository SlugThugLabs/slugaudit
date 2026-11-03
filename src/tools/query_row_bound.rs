//! The cheap size estimate that lets a row be dropped from a page without
//! converting it.

use rusqlite::types::Value as SqlValue;

/// A lower bound on the serialized JSON size of one row, computed from the raw
/// cells without converting them.
///
/// Object punctuation, column names and the quotes around string values are
/// all omitted, as are NULL/integer/real contributions; a TEXT cell counts its
/// UTF-8 byte length (JSON escaping only ever adds bytes) and a BLOB cell
/// counts its hex length. Every omitted term is positive, so this never
/// exceeds the exact `serialized_len(&row_to_json(..))`: a row whose bound
/// already exceeds the page can be dropped without converting it.
pub(super) fn row_serialized_lower_bound(values: &[SqlValue]) -> usize {
    values.iter().fold(0usize, |total, value| {
        let contribution = match value {
            SqlValue::Text(text) => text.len(),
            // KEEP saturating: a lower bound clamped at usize::MAX is still a lower bound, and it is only compared with a page budget it already exceeds
            SqlValue::Blob(bytes) => bytes.len().saturating_mul(2),
            SqlValue::Null | SqlValue::Integer(_) | SqlValue::Real(_) => 0,
        };
        // KEEP saturating: a lower bound clamped at usize::MAX is still a lower bound, and it is only compared with a page budget it already exceeds
        total.saturating_add(contribution)
    })
}
