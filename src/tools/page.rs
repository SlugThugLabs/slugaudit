//! The one paging contract every paged tool returns: `leads` (and the
//! `mechanic add` first count, which is a `leads` response), `query`,
//! `structure` and `finding_read` each embed a [`Page`] as their `page`
//! field, so a caller learns one set of names once.
//!
//! The struct is nested rather than flattened so the shared contract is one
//! visible `$defs` entry in every output schema instead of eight properties
//! repeated (and free to drift) per tool. Tool-specific extras that carry
//! real information (`query`'s `clipped_cells`, `leads`' `files_covered`,
//! `structure`'s second cursor `next_start_after_file`) stay beside it on the
//! tool's own response.

use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::context::internal_error;

/// Converts a SQLite count into [`Page::total`]'s unsigned type. A count is
/// never negative, so a negative one is an internal error, not a clamp.
pub(super) fn total_of(count: i64) -> Result<u64, ErrorData> {
    u64::try_from(count).map_err(|error| internal_error("reading a negative row total", error))
}

/// What cut a page. It reports the bound that acted, never a judgment about
/// whether the page was the right size.
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
    /// `max_query_response_bytes`: the requested (or configured default)
    /// `page_bytes` was at or above it and was clamped. A larger `page_bytes`
    /// returns nothing more; page on with `next_offset`.
    Ceiling,
    /// `structure` only: the per-page match cap `max_structure_matches` was
    /// reached; more matches exist and `next_offset` continues. A larger
    /// `page_bytes` does not lift it.
    MatchCap,
    /// `structure` only: the multi-file scan window ended before the
    /// matching files did (`files_not_scanned` > 0). There is no
    /// `next_offset`; continue with `start_after_file` =
    /// `next_start_after_file` and `offset` 0.
    ScanWindow,
    /// `structure` only: the execution time budget cancelled the parse of
    /// `parse_timed_out_file`. There is no `next_offset`; continue with
    /// `start_after_file` = `next_start_after_file` (that file) and `offset`
    /// 0.
    TimeBudget,
}

/// Whether the scan behind a stored result covered everything it set out to.
/// It is written once with the result, so every page of that result reports
/// the same value, however many calls the caller takes to read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    /// Every file or row the request selects was scanned.
    Complete,
    /// The scan stopped early (`structure` and custom-mechanic scans, which
    /// run under a file window and the call deadline). Continuing is a new
    /// request that syncs, starting after `resume_after_file`.
    Partial {
        /// The bound that stopped the scan (`scan_window` or `time_budget`).
        limited_by: PageLimit,
        /// The last file fully scanned; the new request resumes after it.
        resume_after_file: Option<String>,
        /// Files scanned before the scan stopped.
        files_scanned: u64,
        /// Files the request selects.
        files_total: u64,
    },
}

/// One page of a paged result; every paged tool carries these fields under
/// `page` with the same meaning. To page: while `cursor` is present, repeat
/// the call with only that `cursor`; it names a result stored when the first
/// call ran, so the rows do not move and no `path` is needed. `cursor` is
/// absent exactly when `complete` is true. `next_offset` and
/// `expected_revision` are the pre-cursor mechanism and remain only on the
/// tools not yet moved to cursors; `structure` alone has a second cursor
/// there: when `next_offset` is null and `complete` is false, use its
/// `next_start_after_file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Page {
    /// Items in this page: rows (`leads`, `query`), matches (`structure`),
    /// or findings (`finding_read`).
    pub returned: usize,
    /// Items the whole request selects, ignoring `offset` and the page cut;
    /// never an estimate. Always present; null when the tool cannot count it
    /// (always on `structure`; on `query` when counting ran out of budget).
    /// What it counts is on each tool's `page` property.
    pub total: Option<u64>,
    /// True when nothing remains to page. For `structure` it also means no
    /// scan window remains (no `next_start_after_file`).
    pub complete: bool,
    /// Pre-cursor paging only: always present; null when nothing remains to
    /// page (`complete`), and on `structure` also when only a new scan window
    /// remains. Otherwise pass it as `offset` on the next call. Removed with
    /// the last tool that still uses it.
    pub next_offset: Option<usize>,
    /// The token for the next page, copied exactly into the next call's
    /// `cursor`. Omitted when nothing remains (`complete`) and on tools that
    /// still page by `next_offset`. A cursor does not survive a server
    /// restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Whether the scan behind the stored result covered everything; omitted
    /// on tools that do not store results yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completeness: Option<Completeness>,
    /// The byte budget that applied to this page's items, in serialized JSON
    /// bytes: the request's `page_bytes` or the server default
    /// (`default_page_bytes`, 64 KiB unless configured), after the clamp to
    /// the ceiling `max_query_response_bytes`. `limited_by` = `ceiling` says
    /// the clamp acted on a cut page.
    pub page_bytes: usize,
    /// Serialized JSON bytes of this page's items. The first item is always
    /// returned so paging advances, so this exceeds `page_bytes` only when
    /// that item alone is larger: `leads` sets `oversized_row`, `query` clips
    /// cells (`clipped_cells`, `non_finite_cells`), `finding_read` sets the
    /// row's `detail_clipped`; `structure` has no flag.
    pub bytes_used: usize,
    /// What cut this page; `none` exactly when the result ended.
    pub limited_by: PageLimit,
    /// The index revision every part of this response was read from. Pass it
    /// as `expected_revision` when resuming.
    pub revision_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_serializes_exactly_the_eight_shared_field_names() {
        let page = Page {
            returned: 2,
            total: Some(5),
            complete: false,
            next_offset: Some(2),
            cursor: None,
            completeness: None,
            page_bytes: 100,
            bytes_used: 90,
            limited_by: PageLimit::Ceiling,
            revision_id: "rev-1".to_owned(),
        };

        let value = serde_json::to_value(&page).expect("a page serializes");

        assert_eq!(
            value,
            serde_json::json!({
                "returned": 2,
                "total": 5,
                "complete": false,
                "next_offset": 2,
                "page_bytes": 100,
                "bytes_used": 90,
                "limited_by": "ceiling",
                "revision_id": "rev-1",
            })
        );
    }

    #[test]
    fn a_last_page_serializes_next_offset_and_unknown_total_as_json_null() {
        let page = Page {
            returned: 1,
            total: None,
            complete: true,
            next_offset: None,
            cursor: None,
            completeness: None,
            page_bytes: 100,
            bytes_used: 10,
            limited_by: PageLimit::None,
            revision_id: "rev-1".to_owned(),
        };

        let value = serde_json::to_value(&page).expect("a page serializes");

        assert_eq!(value["next_offset"], serde_json::Value::Null);
        assert_eq!(value["total"], serde_json::Value::Null);
        assert!(
            value
                .as_object()
                .expect("an object")
                .contains_key("next_offset"),
            "the key stays on the wire"
        );
    }

    #[test]
    fn a_negative_count_is_refused_as_a_page_total() {
        let error = total_of(-1).expect_err("a negative count is not a total");

        assert!(
            error.message.contains("negative row total"),
            "{}",
            error.message
        );
        assert_eq!(total_of(7).expect("a count converts"), 7);
    }

    #[test]
    fn every_page_limit_serializes_as_its_snake_case_name() {
        let names = [
            PageLimit::None,
            PageLimit::Limit,
            PageLimit::PageBytes,
            PageLimit::Ceiling,
            PageLimit::MatchCap,
            PageLimit::ScanWindow,
            PageLimit::TimeBudget,
        ]
        .map(|limit| serde_json::to_value(limit).expect("a limit serializes"));

        assert_eq!(
            names,
            [
                "none",
                "limit",
                "page_bytes",
                "ceiling",
                "match_cap",
                "scan_window",
                "time_budget"
            ]
            .map(serde_json::Value::from)
        );
    }
}
