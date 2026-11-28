//! `mechanic` — the auditor registers, removes and lists its own mechanics.
//!
//! The built-ins are counting primitives, not a checklist: what counts as a
//! signal in a codebase is the auditor's decision, and this tool is where it
//! records one. An entry is a Tree-sitter query with exactly one capture over
//! one language; `leads` counts and pages it beside the built-ins, and
//! `report` lists it. Entries are scoped per canonical project root in memory
//! and end when `project_control off` runs or this server exits.

pub(in crate::tools) mod catalog;
mod list;
mod register;
pub(in crate::tools) mod scan;
mod scan_files;

use super::leads::LeadsResponse;
use super::{LegacyPaging, Page, check_bytes_limit, check_optional_path_bytes};
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::progress::ProgressSink;
use crate::sync;
use catalog as mechanic_catalog;
use register::{add, remove};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Generous byte cap on every free-form field before the precise rules
/// (`mechanic_catalog`) run, so an oversized value is never copied or echoed.
const MAX_FIELD_BYTES: usize = 64 * 1024;

/// Where a mechanic comes from. Listings are ordered session, then builtin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MechanicSource {
    /// Registered with `mechanic add` on this server; ends when it exits.
    Session,
    /// Shipped with SlugAudit; can never be removed or overridden.
    Builtin,
}

/// One mechanic in the catalogue.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MechanicListing {
    /// The id `leads` takes as `mechanic`.
    pub id: String,
    /// Where the entry comes from.
    pub source: MechanicSource,
    /// The author's note; `null` for a built-in.
    pub note: Option<String>,
    /// The grammar a registered entry runs on; `null` for a built-in.
    pub language: Option<String>,
    /// A registered entry's Tree-sitter query; `null` for a built-in.
    pub query: Option<String>,
}

/// What a `mechanic` call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MechanicAction {
    /// Register `{id, language, query, note}` and return its first count.
    Add,
    /// Remove the registered entry `id`.
    Remove,
    /// List every mechanic for this project.
    List,
}

#[derive(Debug, Deserialize, JsonSchema)]
/// One `mechanic` call: the action, the entry for `add`, the id for
/// `remove`, and the first page's size for `add`.
pub struct MechanicRequest {
    /// Any path inside the active project. Required unless `cursor` is given.
    #[serde(default)]
    pub path: Option<String>,
    /// `add`, `remove`, or `list`. Required unless `cursor` is given; ignored
    /// when `cursor` is given: a cursor only ever continues a stored `list`,
    /// and never registers or removes anything.
    #[serde(default)]
    pub action: Option<MechanicAction>,
    /// `list` only: the `page.cursor` of the previous page, copied exactly.
    /// It continues the list as it was when the first page was read, without
    /// a `path`; every other field except `page_bytes` is ignored. To page
    /// the first count `add` returns, use that response's own `page.cursor`
    /// with `leads`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Refuses `offset` and `expected_revision` with a message
    /// that names `cursor`.
    #[serde(flatten, default)]
    #[schemars(skip)]
    pub legacy_paging: LegacyPaging,
    /// `add`/`remove`: the entry id, 1-64 characters of `a-z`, `0-9`, `_`.
    /// It can never be a built-in's id or one already registered.
    #[serde(default)]
    pub id: Option<String>,
    /// `add`: the grammar the query compiles against and the indexed files
    /// it reads: exactly the lowercase name the index stores for a file
    /// (`python`, `rust`, `javascript`, `tsx`), never a capitalised name or
    /// a pack alias such as `shell` (use `bash`). Prose is refused.
    #[serde(default)]
    pub language: Option<String>,
    /// `add`: a Tree-sitter S-expression with exactly one capture, e.g.
    /// `(call function: (identifier) @callee (#eq? @callee "eval"))`. Each
    /// distinct captured node is one row. At most `max_structure_query_bytes`
    /// bytes (16384 by default). A query that does not compile or whose trial
    /// scan is incomplete is refused.
    #[serde(default)]
    pub query: Option<String>,
    /// `add`: what this entry counts, in your words (1-1024 bytes). It is
    /// listed beside the entry in `leads`, `report` and `list`.
    #[serde(default)]
    pub note: Option<String>,
    /// `add`: maximum rows in the first page, as `leads`' `limit`.
    #[serde(default)]
    pub limit: Option<usize>,
    /// `add`: first page size in serialized JSON bytes, as `leads`'
    /// `page_bytes`. `list`: the size of each page of listings.
    #[serde(default)]
    pub page_bytes: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
/// The catalogue after the call, and for `add` the new entry's first count.
pub struct MechanicResponse {
    /// The action that ran.
    pub action: MechanicAction,
    /// `add`/`remove`: the id the action applied to; `null` for `list`.
    pub id: Option<String>,
    /// `add`/`remove`: where that entry lives (or lived).
    pub source: Option<MechanicSource>,
    /// Every mechanic after the call: session entries for this project (oldest
    /// first), then the built-ins.
    pub mechanics: Vec<MechanicListing>,
    /// `add` only: the response `leads` returns for `mechanic` = `id` at the
    /// current revision — complete counts (`mechanic_counts`: the built-ins
    /// and the new entry with its `count`, `files` and `covers`), the first
    /// page of `rows` under the shared `page` contract (`page.total`,
    /// `page.cursor`, `page.revision_id`), `files_total`, `files_covered` and
    /// `files_not_parsed`. Page on with `leads`, passing `page.cursor`.
    pub first_count: Option<LeadsResponse>,
    /// `list` only: the shared paging block for `mechanics`. While
    /// `page.cursor` is present, repeat the call with only that `cursor`;
    /// every page reports the `revision_id` and `mechanics` order of page one.
    /// `add` and `remove` return the whole catalogue and no `page`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Page>,
}

/// Adds, removes, or lists auditor-registered mechanics.
///
/// # Errors
///
/// `invalid_params` for an inactive project, a missing or oversized field, an
/// id that is a built-in's or already registered, a query that does not
/// compile, does not name exactly one capture, or produces an incomplete trial
/// scan, an unknown or prose language, a full session list, or a `remove` of a
/// built-in or an unknown id; an internal error for a failed sync or read.
pub fn mechanic(
    request: &Parameters<MechanicRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<MechanicResponse>, ErrorData> {
    let request = &request.0;
    request.legacy_paging.refuse()?;
    validate(request)?;
    if request.cursor.is_some() {
        return list::list(request, sink, manager).map(Json);
    }
    let path = request.path.as_deref().ok_or_else(|| {
        invalid_params_with_code(
            "path is required when no cursor is given",
            ErrorCode::InvalidInput,
        )
    })?;
    let action = request.action.ok_or_else(|| {
        invalid_params_with_code(
            "action is required when no cursor is given",
            ErrorCode::InvalidInput,
        )
    })?;
    let root = super::project_root(path)?;
    let limits = super::limits_for_root(&root)?;
    let (id, source, first_count) = match action {
        MechanicAction::List => return list::list(request, sink, manager).map(Json),
        MechanicAction::Add => {
            let (id, source, first_count) =
                add(request, path, root.as_path(), &limits, sink, manager)?;
            (Some(id), Some(source), Some(first_count))
        }
        MechanicAction::Remove => {
            let (id, source) = remove(request, path, root.as_path(), sink, manager)?;
            (Some(id), Some(source), None)
        }
    };
    let catalog = mechanic_catalog::load(manager.session_mechanics(), root.as_path())?;
    tracing::info!(
        action = ?action,
        source = ?source,
        registered = catalog.custom.len(),
        "mechanic call completed"
    );
    Ok(Json(MechanicResponse {
        action,
        id,
        source,
        mechanics: catalog.listings(),
        first_count,
        page: None,
    }))
}

fn validate(request: &MechanicRequest) -> Result<(), ErrorData> {
    check_optional_path_bytes(request.path.as_deref())?;
    for (name, value) in [
        ("id", &request.id),
        ("language", &request.language),
        ("query", &request.query),
        ("note", &request.note),
    ] {
        if let Some(text) = value.as_deref() {
            check_bytes_limit(text, name, MAX_FIELD_BYTES)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod refusal_tests;

#[cfg(test)]
mod paged_tests;

#[cfg(test)]
mod scan_tests;

#[cfg(test)]
mod scan_stop_tests;
