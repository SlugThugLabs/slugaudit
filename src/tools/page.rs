// slugaudit-line-exception: approved-by=slugthug; reason=the one paging contract: Page, PageLimit, Completeness and the legacy-paging refusal are one wire contract and must stay together
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

use crate::error_codes::{ErrorCode, payload};

/// The pre-cursor paging parameters, kept only so a request that still sends
/// them is refused with a message instead of the parameters being silently
/// dropped. A request embeds it as
/// `#[serde(flatten, default)] #[schemars(skip)] pub legacy_paging: LegacyPaging`,
/// so the fields are accepted on input and absent from the tool schema. The
/// values are kept as raw JSON: only their presence matters.
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
    /// `invalid_params` with the `paging_parameter_removed` code when either
    /// field is present (including an explicit JSON null).
    pub fn refuse(&self) -> Result<(), ErrorData> {
        if self.offset.is_none() && self.expected_revision.is_none() {
            return Ok(());
        }
        Err(ErrorData::invalid_params(
            "offset and expected_revision were removed from this tool; page with the cursor",
            payload(ErrorCode::PagingParameterRemoved),
        ))
    }
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
    /// returns nothing more; page on with `cursor`.
    Ceiling,
    /// `structure` only: the per-page match cap `max_structure_matches` was
    /// reached; more matches exist and `cursor` continues. A larger
    /// `page_bytes` does not lift it.
    MatchCap,
    /// `structure` only: the multi-file scan window ended before the
    /// matching files did (`files_not_scanned` > 0). Read the stored result to
    /// its end with `cursor`, then make a new request with `start_after_file`
    /// = `next_start_after_file`.
    ScanWindow,
    /// `structure` only: the execution time budget cancelled the parse of
    /// `parse_timed_out_file`. Read the stored result to its end with
    /// `cursor`, then make a new request with `start_after_file` =
    /// `next_start_after_file` (that file).
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
/// the call with only that `cursor` (plus `page_bytes`, and `limit` on
/// `leads`, to resize the page); it names a result stored when the first call
/// ran, so the rows do not move, `revision_id` is that of the first call, and
/// no `path` is needed. `cursor` is absent exactly when `complete` is true.
/// `structure` alone has a second step after the last page: when `completeness`
/// is `partial`, make a new request with `start_after_file` =
/// `next_start_after_file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Page {
    /// Items in this page: rows (`leads`, `query`), matches (`structure`),
    /// or findings (`finding_read`).
    pub returned: usize,
    /// Items the whole request selects, ignoring the page cut;
    /// never an estimate. Always present; null when the tool cannot count it
    /// (always on `structure`; on `query` when counting ran out of budget).
    /// What it counts is on each tool's `page` property.
    pub total: Option<u64>,
    /// True when nothing remains to page. For `structure` it also means no
    /// scan window remains (no `next_start_after_file`).
    pub complete: bool,
    /// The token for the next page, copied exactly into the next call's
    /// `cursor`. Omitted when nothing remains (`complete`). A cursor does not
    /// survive a server restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Whether the scan behind the stored result covered everything; omitted
    /// when the tool reports no scan.
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
    /// The index revision every part of this response was read from: that of
    /// the call that made the result, on every page.
    pub revision_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_paging_refuses_an_offset_with_the_cursor_remedy() {
        let legacy: LegacyPaging =
            serde_json::from_value(serde_json::json!({ "offset": 5 })).expect("parses");

        let error = legacy.refuse().expect_err("offset is removed");

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        let data = error.data.expect("a payload");
        assert_eq!(data["code"], "paging_parameter_removed");
        assert!(data["remedy"].as_str().expect("remedy").contains("cursor"));
    }

    #[test]
    fn legacy_paging_refuses_an_expected_revision_with_the_cursor_remedy() {
        let legacy: LegacyPaging =
            serde_json::from_value(serde_json::json!({ "expected_revision": "r1" }))
                .expect("parses");

        let error = legacy.refuse().expect_err("expected_revision is removed");

        let data = error.data.expect("a payload");
        assert_eq!(data["code"], "paging_parameter_removed");
        assert!(data["remedy"].as_str().expect("remedy").contains("cursor"));
    }

    #[test]
    fn legacy_paging_accepts_a_request_with_neither_field() {
        let legacy: LegacyPaging = serde_json::from_value(serde_json::json!({})).expect("parses");

        legacy.refuse().expect("nothing to refuse");
    }

    #[test]
    fn legacy_paging_flattened_into_a_request_is_read_but_not_in_its_schema() {
        #[derive(Deserialize, JsonSchema)]
        struct Probe {
            #[serde(default)]
            path: Option<String>,
            #[serde(flatten, default)]
            #[schemars(skip)]
            legacy_paging: LegacyPaging,
        }

        let probe: Probe = serde_json::from_value(serde_json::json!({ "path": "p", "offset": 1 }))
            .expect("parses");
        let schema = serde_json::to_string(&schemars::schema_for!(Probe)).expect("schema");

        assert_eq!(probe.path.as_deref(), Some("p"));
        probe.legacy_paging.refuse().expect_err("offset was sent");
        assert!(!schema.contains("offset"), "{schema}");
        assert!(!schema.contains("expected_revision"), "{schema}");
        assert!(!schema.contains("legacy_paging"), "{schema}");
    }

    #[test]
    fn a_page_serializes_exactly_the_seven_shared_field_names() {
        let page = Page {
            returned: 2,
            total: Some(5),
            complete: false,
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
                "page_bytes": 100,
                "bytes_used": 90,
                "limited_by": "ceiling",
                "revision_id": "rev-1",
            })
        );
    }

    #[test]
    fn a_last_page_serializes_unknown_total_as_json_null_and_has_no_next_offset() {
        let page = Page {
            returned: 1,
            total: None,
            complete: true,
            cursor: None,
            completeness: None,
            page_bytes: 100,
            bytes_used: 10,
            limited_by: PageLimit::None,
            revision_id: "rev-1".to_owned(),
        };

        let value = serde_json::to_value(&page).expect("a page serializes");

        assert_eq!(value["total"], serde_json::Value::Null);
        assert!(
            !value
                .as_object()
                .expect("an object")
                .contains_key("next_offset"),
            "the pre-cursor field is gone from the wire"
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
