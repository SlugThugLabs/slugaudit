//! Shared paging wire contract (`Page`, `PageLimit`, `Completeness`,
//! `LegacyPaging`) and effective page-size resolution (`PageSize`) across
//! `leads`, `query`, `structure`, `finding_read`, and `mechanic list`.

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::ResourceLimits;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The removed `offset` and `expected_revision` paging parameters, accepted
/// only so a request that sends them is refused with `paging_parameter_removed`
/// instead of silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LegacyPaging {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_revision: Option<serde_json::Value>,
}

impl LegacyPaging {
    /// Refuses a request that carries `offset` or `expected_revision`.
    ///
    /// # Errors
    ///
    /// `invalid_params` with `paging_parameter_removed` when either field is present.
    pub fn refuse(&self) -> Result<(), ErrorData> {
        if self.offset.is_none() && self.expected_revision.is_none() {
            return Ok(());
        }
        Err(invalid_params_with_code(
            "offset and expected_revision were removed from this tool; page with the cursor",
            ErrorCode::PagingParameterRemoved,
        ))
    }
}

/// Effective page byte budget resolved for a paged tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tools) struct PageSize {
    /// Effective page size in serialized JSON bytes.
    pub(in crate::tools) bytes: usize,
    /// True when the requested (or default) page size was clamped to the ceiling.
    pub(in crate::tools) at_ceiling: bool,
}

impl PageSize {
    /// What a page cut by this byte budget reports as its `limited_by`.
    #[cfg(test)]
    pub(super) fn byte_cut(self) -> PageLimit {
        if self.at_ceiling {
            PageLimit::Ceiling
        } else {
            PageLimit::PageBytes
        }
    }

    /// Refuses `page_bytes: 0`.
    pub(in crate::tools) fn refuse_zero(requested: Option<usize>) -> Result<(), ErrorData> {
        if requested == Some(0) {
            return Err(invalid_params_with_code(
                "page_bytes must be at least 1; omit it for the default page size",
                ErrorCode::InvalidInput,
            ));
        }
        Ok(())
    }

    /// Resolves the effective page byte budget against `limits`.
    pub(in crate::tools) fn resolve(
        requested: Option<usize>,
        limits: &ResourceLimits,
    ) -> Result<Self, ErrorData> {
        Self::refuse_zero(requested)?;
        let wanted = match requested {
            Some(bytes) => bytes,
            None => limits.default_page_bytes,
        };
        let ceiling = limits.max_query_response_bytes;
        Ok(Self {
            bytes: wanted.min(ceiling),
            at_ceiling: wanted >= ceiling,
        })
    }
}

/// What cut a page or stopped an underlying scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageLimit {
    /// Nothing: the result ended on this page and `complete` is true.
    None,
    /// `leads` only: the caller's `limit` was reached; more rows exist.
    Limit,
    /// The byte budget `page_bytes` was reached; more items exist. A larger
    /// `page_bytes` (up to the server ceiling) returns a larger page.
    PageBytes,
    /// The byte budget was reached and that budget is the server ceiling
    /// `max_query_response_bytes`. Page on with `cursor`.
    Ceiling,
    /// `structure` only: the per-page match cap `max_structure_matches` cut
    /// this page; repeat the call with `page.cursor` to read the next page of
    /// stored matches.
    MatchCap,
    /// `structure` only: the multi-file scan window ended before the
    /// matching files did (`files_not_scanned` > 0). Read the stored result to
    /// its end with `cursor`, then make a new request with `start_after_file`
    /// = `next_start_after_file`.
    ScanWindow,
    /// `structure` or custom-mechanic scan: the execution time budget stopped
    /// the scan before every file finished. Read the stored result with
    /// `cursor`, then resume after `resume_after_file` (or `next_start_after_file`).
    TimeBudget,
    /// `structure` or custom-mechanic scan: Tree-sitter's per-file `QueryCursor`
    /// match-state limit (`65,536` states) was exceeded on a file, so some
    /// matches were dropped (`completeness` is `partial`). Narrow the Tree-sitter
    /// pattern so no match states overflow.
    QueryMatchLimit,
}

/// Whether the scan behind a stored result covered everything it set out to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    /// Every file or row the request selects was scanned without overflow.
    Complete,
    /// The scan stopped early or hit a pattern-state limit (`structure` and
    /// custom-mechanic scans). Continuing is a new request that syncs, starting
    /// after `resume_after_file`.
    Partial {
        /// The bound that made the scan partial (`scan_window`, `time_budget`,
        /// or `query_match_limit`).
        limited_by: PageLimit,
        /// The last file fully scanned without overflow; the new request resumes after it.
        resume_after_file: Option<String>,
        /// Files scanned cleanly before the scan stopped or overflowed.
        files_scanned: u64,
        /// Files the request selects.
        files_total: u64,
    },
}

/// One page of a paged result; every paged tool carries these fields under
/// `page` with the same meaning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Page {
    /// Items in this page: rows (`leads`, `query`), matches (`structure`),
    /// findings (`finding_read`) or mechanics (`mechanic list`).
    pub returned: usize,
    /// Items the whole request selects, ignoring the page cut;
    /// never an estimate. Null when the tool cannot count it (`structure`).
    pub total: Option<u64>,
    /// True when nothing remains to page and no underlying scan is partial.
    pub complete: bool,
    /// The token for the next page, copied into the next call's `cursor`.
    /// Omitted when nothing remains in this stored result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Whether the scan behind the stored result covered everything; omitted
    /// when the tool reports no scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completeness: Option<Completeness>,
    /// Effective byte budget that applied to this page's items.
    pub page_bytes: usize,
    /// Serialized JSON bytes of this page's items.
    pub bytes_used: usize,
    /// What cut this page (or, on the final page of `structure`, what made the scan partial).
    pub limited_by: PageLimit,
    /// Index revision every part of this response was read from.
    pub revision_id: String,
    /// Continuation calls only (`cursor` supplied): the `revision_id`
    /// currently on disk in `project.db` when this page was served, if readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_disk_revision_id: Option<String>,
    /// Present only when this call discarded and rebuilt the project database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_discard: Option<crate::sync::DatabaseDiscard>,
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
