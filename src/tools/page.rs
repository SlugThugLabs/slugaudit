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

use schemars::JsonSchema;
use serde::Serialize;

/// What cut a page. It reports the bound that acted, never a judgment about
/// whether the page was the right size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
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
    /// reached; more matches exist and `next_offset` continues.
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

/// One page of a paged result: how much came back, how much there is, how
/// the page was cut, and how to get the next one. Every paged tool carries
/// exactly these fields under `page`, with the same meaning.
///
/// To page: while `complete` is false and `next_offset` is present, repeat
/// the call with `offset` = `next_offset` and `expected_revision` =
/// `revision_id`; a changed index is then refused with `stale_revision`
/// instead of silently skipping or repeating items. `structure` alone has a
/// second cursor beside this struct: when `complete` is false and
/// `next_offset` is absent, repeat with `start_after_file` = the response's
/// `next_start_after_file` and `offset` 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Page {
    /// Items in this page: rows (`leads`, `query`), matches (`structure`),
    /// or findings (`finding_read`).
    pub returned: usize,
    /// Items the whole request selects, ignoring `offset` and the page cut;
    /// never an estimate. `leads`: the rows of the requested mechanic (or of
    /// every mechanic in a compact whole-surface call; `0` for a counts-only
    /// call). `query`: the exact row count SQLite computed over the same
    /// statement, null only when counting ran out of its budget (the
    /// response's `total_unavailable` says why). `finding_read`: this
    /// session's findings under the `file` filter. `structure`: always null,
    /// because counting the matches means running the whole scan.
    pub total: Option<i64>,
    /// True when nothing remains to page: the result ends on this page. For
    /// `structure` it also means no scan window remains (no
    /// `next_start_after_file`). False means more remain: follow
    /// `next_offset`, or for `structure` without one, its
    /// `next_start_after_file`.
    pub complete: bool,
    /// Present only when more items remain in this result: pass it as
    /// `offset` on the next call (same other arguments) together with
    /// `expected_revision` = `revision_id`.
    pub next_offset: Option<usize>,
    /// The byte budget that applied to this page's items, in serialized JSON
    /// bytes: the request's `page_bytes` or the server default
    /// (`default_page_bytes`, 64 KiB unless configured), after the clamp to
    /// the ceiling `max_query_response_bytes`. `limited_by` = `ceiling` says
    /// the clamp acted on a cut page.
    pub page_bytes: usize,
    /// Serialized JSON bytes of this page's items, measured per item. The
    /// first item is always returned so paging advances, so this exceeds
    /// `page_bytes` only when that first item alone is larger (see the tool's
    /// `oversized_row` or `clipped_cells`). The rest of the response envelope
    /// is additional.
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
