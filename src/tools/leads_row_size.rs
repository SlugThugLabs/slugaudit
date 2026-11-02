//! The size of a stored `leads` row as the response carries it, which is what
//! the page budget counts.

use super::StoredLead;
use crate::tools::context::internal_error;
use crate::util::serialized_len;
use rmcp::ErrorData;
use serde::Serialize;

impl StoredLead {
    /// Serialized JSON bytes of this row as the response carries it, measured
    /// on a borrowed view with the same fields as [`CompactLeadRow`] or
    /// [`LeadRow`], without copying the row.
    pub(in crate::tools) fn response_bytes(&self) -> Result<usize, ErrorData> {
        #[derive(Serialize)]
        struct CompactView<'a> {
            mechanic: &'a str,
            file: &'a str,
            line: i64,
        }
        #[derive(Serialize)]
        struct FullView<'a> {
            file: &'a str,
            line: i64,
            detail: &'a str,
            detail_clipped: bool,
        }
        if self.is_compact() {
            serialized_len(&CompactView {
                mechanic: &self.mechanic,
                file: &self.file,
                line: self.line,
            })
        } else {
            serialized_len(&FullView {
                file: &self.file,
                line: self.line,
                detail: &self.detail,
                detail_clipped: self.detail_clipped,
            })
        }
        .map_err(|error| internal_error("measuring a leads row", error))
    }
}
