//! `mechanic` — the auditor registers, removes and lists its own mechanics.
//!
//! The built-ins are counting primitives, not a checklist: what counts as a
//! signal in a codebase is the auditor's decision, and this tool is where it
//! records one. An entry is a Tree-sitter query with exactly one capture over
//! one language; `leads` counts and pages it beside the built-ins, and
//! `report` lists it. An entry lives in this server's memory and ends with
//! the process. No entry can remove, rename or override a built-in.

use super::leads::LeadsResponse;
use super::mechanic_catalog;
use super::mechanic_register::{add, remove};
use super::{LegacyPaging, Page};
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_PATH_BYTES: usize = 32 * 1024;
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

/// A registered entry that could not be loaded, with the reason. It is not
/// counted by `leads` until it is removed, and it is never dropped from this
/// list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InvalidMechanic {
    /// `session`: the entry was registered on this server.
    pub source: MechanicSource,
    /// Position in the session list (oldest first).
    pub index: Option<usize>,
    /// The entry's id, when it has one.
    pub id: Option<String>,
    /// Why it was not loaded: a compile error, a broken field rule, or an id
    /// collision.
    pub error: String,
}

/// What a `mechanic` call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MechanicAction {
    /// Register `{id, language, query, note}` and return its first count.
    Add,
    /// Remove the registered entry `id`.
    Remove,
    /// List every mechanic and every entry that failed to load.
    List,
}

#[derive(Debug, Deserialize, JsonSchema)]
/// One `mechanic` call: the action, the entry for `add`, the id for
/// `remove`, and the first page's size for `add`.
pub struct MechanicRequest {
    /// Any path inside the active project. Required unless `cursor` is given.
    #[serde(default)]
    pub path: Option<String>,
    /// `add`, `remove`, or `list`. Ignored when `cursor` is given: a cursor
    /// only ever continues a stored `list`, and never registers or removes
    /// anything.
    pub action: MechanicAction,
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
    /// bytes (16384 by default). A query that does not compile is refused
    /// with the compiler's message.
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
    /// `add`/`remove`: the id the action applied to; `null` for a `remove`
    /// by `index` whose entry had no string id.
    pub id: Option<String>,
    /// `add`/`remove`: where that entry lives (or lived).
    pub source: Option<MechanicSource>,
    /// Every mechanic after the call: session entries (oldest first), then
    /// the built-ins.
    pub mechanics: Vec<MechanicListing>,
    /// Registered entries that failed to load, with the reason.
    pub invalid: Vec<InvalidMechanic>,
    /// `invalid.len()`.
    pub mechanics_invalid: usize,
    /// `add` only: the response `leads` returns for `mechanic` = `id` at the
    /// current revision — complete counts (`mechanic_counts`: the built-ins
    /// and the new entry with its `count`, `files` and `covers`), the first
    /// page of `rows` under the shared `page` contract (`page.total`,
    /// `page.cursor`, `page.revision_id`), `files_total`,
    /// `files_covered` and `files_not_parsed`. As in `leads`, `files_total`
    /// is the number of files holding at least one row, not the files read
    /// (those are `covers` and the index): a query with no match has
    /// `page.total`, `files_total` and `files_covered` all `0`. Page on with
    /// `leads`, passing `page.cursor`.
    pub first_count: Option<LeadsResponse>,
    /// `list` only: the shared paging block for `mechanics`. While
    /// `page.cursor` is present, repeat the call with only that `cursor`;
    /// every page reports the `revision_id`, `invalid` and `mechanics`
    /// order of page one. `add` and `remove` return the whole catalogue and
    /// no `page`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Page>,
}

/// Adds, removes, or lists auditor-registered mechanics.
///
/// # Errors
///
/// `invalid_params` for an inactive project, a missing or oversized field, an
/// id that is a built-in's or already registered, a query that does not
/// compile or does not name exactly one capture, an unknown or prose
/// language, a full session list, or a `remove` of a built-in or an unknown
/// id; an internal error for a failed sync or a failed first count.
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
        ErrorData::invalid_params("path is required when no cursor is given", None)
    })?;
    let root = super::project_root(path)?;
    let limits = super::limits_for_root(&root)?;
    let (id, source, first_count) = match request.action {
        MechanicAction::List => return list::list(request, sink, manager).map(Json),
        MechanicAction::Add => {
            let (id, source, first_count) = add(request, path, &limits, sink, manager)?;
            (Some(id), Some(source), Some(first_count))
        }
        MechanicAction::Remove => {
            let (id, source) = remove(request, manager)?;
            (Some(id), Some(source), None)
        }
    };
    let catalog = mechanic_catalog::load(manager.session_mechanics(), &limits);
    tracing::info!(
        action = ?request.action,
        source = ?source,
        registered = catalog.custom.len(),
        mechanics_invalid = catalog.invalid.len(),
        "mechanic call completed"
    );
    Ok(Json(MechanicResponse {
        action: request.action,
        id,
        source,
        mechanics: catalog.listings(),
        mechanics_invalid: catalog.invalid.len(),
        invalid: catalog.invalid,
        first_count,
        page: None,
    }))
}

fn validate(request: &MechanicRequest) -> Result<(), ErrorData> {
    if request
        .path
        .as_deref()
        .is_some_and(|path| crate::util::exceeds_input_bytes(path, MAX_PATH_BYTES))
    {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    for (name, value) in [
        ("id", &request.id),
        ("language", &request.language),
        ("query", &request.query),
        ("note", &request.note),
    ] {
        if value
            .as_deref()
            .is_some_and(|value| crate::util::exceeds_input_bytes(value, MAX_FIELD_BYTES))
        {
            return Err(ErrorData::invalid_params(
                format!("{name} exceeds {MAX_FIELD_BYTES} bytes"),
                None,
            ));
        }
    }
    Ok(())
}

#[path = "mechanic_list.rs"]
mod list;

#[cfg(test)]
#[path = "mechanic_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mechanic_paged_tests.rs"]
mod paged_tests;

#[cfg(test)]
#[path = "mechanic_scan_tests.rs"]
mod scan_tests;
