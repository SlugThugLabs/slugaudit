//! `mechanic` — the auditor registers, removes and lists its own mechanics.
//!
//! The built-ins are counting primitives, not a checklist: what counts as a
//! signal in a codebase is the auditor's decision, and this tool is where it
//! records one. An entry is a Tree-sitter query with exactly one capture over
//! one language; `leads` counts and pages it beside the built-ins, and
//! `report` lists it. A session entry lives in this server's memory and ends
//! with the process; `persist: true` writes it to the project's
//! `.planning/slugaudit/config.json` instead, where every later call (and
//! every later server) reads it. No entry can remove, rename or override a
//! built-in.

use super::leads::LeadsResponse;
use super::mechanic_catalog;
use super::mechanic_register::{add, remove};
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

/// Where a mechanic comes from. Listings are ordered session, then
/// project_config, then builtin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MechanicSource {
    /// Registered with `mechanic add` on this server; ends when it exits.
    Session,
    /// Persisted in the project's `.planning/slugaudit/config.json`.
    ProjectConfig,
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
/// counted by `leads` until it is fixed or removed, and it is never dropped
/// from this list.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InvalidMechanic {
    /// `project_config` for a persisted entry (or the whole section);
    /// `session` for a session entry whose grammar no longer loads.
    pub source: MechanicSource,
    /// Position in `mechanics.entries` (or in the session list); `null` when
    /// the whole section or file failed.
    pub index: Option<usize>,
    /// The entry's id, when it has one.
    pub id: Option<String>,
    /// Why it was not loaded: a bad JSON shape, an unknown `version`, a
    /// compile error, a broken field rule, or an id collision.
    pub error: String,
}

/// What a `mechanic` call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MechanicAction {
    /// Register `{id, language, query, note}` and return its first count.
    Add,
    /// Remove the session or persisted entry `id`, or the invalid persisted
    /// entry at `index`.
    Remove,
    /// List every mechanic and every entry that failed to load.
    List,
}

#[derive(Debug, Deserialize, JsonSchema)]
/// One `mechanic` call: the action, the entry for `add`, the id for
/// `remove`, and the first page's size for `add`.
pub struct MechanicRequest {
    /// Any path inside the active project.
    pub path: String,
    /// `add`, `remove`, or `list`.
    pub action: MechanicAction,
    /// `add`/`remove`: the entry id, 1-64 characters of `a-z`, `0-9`, `_`.
    /// It can never be a built-in's id or one already registered.
    #[serde(default)]
    pub id: Option<String>,
    /// `remove` only, instead of `id`: the `index` of a persisted entry
    /// listed in `invalid` (`report.mechanics.invalid`, or `list`) — the way
    /// to remove an entry with no string id or with a built-in's id. Only an
    /// entry that still fails to load is removed this way; a valid one is
    /// refused (remove it by `id`). Passing both `id` and `index` is refused.
    #[serde(default)]
    pub index: Option<usize>,
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
    /// `add`: `true` writes the entry to the project's
    /// `.planning/slugaudit/config.json` (every other key kept) so later
    /// sessions see it; `false` (default) keeps it in this server's memory
    /// until the server exits.
    #[serde(default)]
    pub persist: bool,
    /// `add`: maximum rows in the first page, as `leads`' `limit`.
    #[serde(default)]
    pub limit: Option<usize>,
    /// `add`: first page size in serialized JSON bytes, as `leads`'
    /// `page_bytes`.
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
    /// project_config entries (file order), then the built-ins.
    pub mechanics: Vec<MechanicListing>,
    /// Registered entries that failed to load, with the reason.
    pub invalid: Vec<InvalidMechanic>,
    /// `invalid.len()`.
    pub mechanics_invalid: usize,
    /// `add` only: the response `leads` returns for `mechanic` = `id` at the
    /// current revision — complete counts (`mechanic_counts`: the built-ins
    /// and the new entry with its `count`, `files` and `covers`), the first
    /// page of `rows` under the shared `page` contract (`page.total`,
    /// `page.next_offset`, `page.revision_id`), `files_total`,
    /// `files_covered` and `files_not_parsed`. As in `leads`, `files_total`
    /// is the number of files holding at least one row, not the files read
    /// (those are `covers` and the index): a query with no match has
    /// `page.total`, `files_total` and `files_covered` all `0`. Page on with
    /// `leads`, passing `page.next_offset` and `page.revision_id`.
    pub first_count: Option<LeadsResponse>,
}

/// Adds, removes, or lists auditor-registered mechanics.
///
/// # Errors
///
/// `invalid_params` for an inactive project, a missing or oversized field, an
/// id that is a built-in's or already registered, a query that does not
/// compile or does not name exactly one capture, an unknown or prose
/// language, a full session or project list, a `remove` of a built-in or an
/// unknown id, a `remove` by an `index` that is not an invalid persisted
/// entry, or a `config.json` that is a symlink or cannot be extended; an
/// internal error for a failed read or write of `config.json`, a
/// `config.json` locked by another process past the bounded wait, a failed
/// sync, or a failed first count.
pub fn mechanic(
    request: &Parameters<MechanicRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<MechanicResponse>, ErrorData> {
    let request = &request.0;
    validate(request)?;
    let root = super::project_root(&request.path)?;
    let limits = super::limits_for_root(&root);
    let (id, source, first_count) = match request.action {
        MechanicAction::List => (None, None, None),
        MechanicAction::Add => {
            let (id, source, first_count) = add(request, &root, &limits, sink, manager)?;
            (Some(id), Some(source), Some(first_count))
        }
        MechanicAction::Remove => {
            let (id, source) = remove(request, &root, &limits, manager)?;
            (id, Some(source), None)
        }
    };
    let catalog = mechanic_catalog::load(&root, manager.session_mechanics(), &limits);
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
    }))
}

fn validate(request: &MechanicRequest) -> Result<(), ErrorData> {
    if crate::util::exceeds_input_bytes(&request.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    if request.index.is_some() && request.id.is_some() {
        return Err(ErrorData::invalid_params(
            "`remove` takes `id` or `index`, not both",
            None,
        ));
    }
    if request.index.is_some() && request.action != MechanicAction::Remove {
        return Err(ErrorData::invalid_params(
            "`index` is only for `remove`",
            None,
        ));
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

#[cfg(test)]
#[path = "mechanic_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mechanic_persist_tests.rs"]
mod persist_tests;

#[cfg(test)]
#[path = "mechanic_scan_tests.rs"]
mod scan_tests;
