//! `mechanic add` and `mechanic remove`: the registration rules and the
//! writes they guard. Kept apart from `mechanic.rs` (the tool contract) because
//! this is the only code that changes the registry — the session list — so
//! every check that must hold before a change (built-in ids, collisions,
//! caps) sits beside the change it guards.

use super::leads::{LeadMode, LeadsRequest, LeadsResponse, leads_with_catalog};
use super::mechanic::{MechanicRequest, MechanicSource};
use super::mechanic_catalog::{self, Catalog, Registered, is_builtin};
use crate::error_codes::{self, ErrorCode};
use crate::progress::ProgressSink;
use crate::project::{MAX_SESSION_MECHANICS, MechanicEntry};
use crate::sync;
use rmcp::ErrorData;

fn required(value: Option<&String>, name: &str, action: &str) -> Result<String, ErrorData> {
    value
        .cloned()
        .ok_or_else(|| ErrorData::invalid_params(format!("`{action}` needs `{name}`"), None))
}

fn refused(reason: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(format!("mechanic not added: {reason}"), None)
}

/// Refuses `id` when it is a built-in's or already registered for this
/// session, or when the session list is full.
fn check_free(session: &[MechanicEntry], id: &str) -> Result<(), ErrorData> {
    if is_builtin(id) {
        return Err(refused(format!(
            "`{id}` is a built-in mechanic's id; a registered mechanic can never remove, rename \
             or override a built-in"
        )));
    }
    if session.iter().any(|entry| entry.id == id) {
        return Err(refused(format!(
            "`{id}` is already registered for this session"
        )));
    }
    if session.len() >= MAX_SESSION_MECHANICS {
        return Err(refused(format!(
            "this session already holds {MAX_SESSION_MECHANICS} mechanics, the cap"
        )));
    }
    Ok(())
}

/// Compiles and checks the entry, counts it with the candidate placed where
/// it will live, then registers it — re-checking under the session lock, so
/// nothing is registered when the count fails and a racing `add` of the same
/// id loses cleanly.
pub(super) fn add(
    request: &MechanicRequest,
    path: &str,
    limits: &crate::model::ResourceLimits,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<(String, MechanicSource, LeadsResponse), ErrorData> {
    let entry = MechanicEntry {
        id: required(request.id.as_ref(), "id", "add")?,
        language: required(request.language.as_ref(), "language", "add")?,
        query: required(request.query.as_ref(), "query", "add")?,
        note: required(request.note.as_ref(), "note", "add")?,
    };
    let query = mechanic_catalog::compile(&entry, limits).map_err(refused)?;
    let session = manager.session_mechanics();
    session.with_entries(|entries| check_free(entries, &entry.id))??;
    let mut catalog = mechanic_catalog::load(session, limits)?;
    insert_candidate(&mut catalog, entry.clone(), query);
    let first = LeadsRequest {
        path: Some(path.to_owned()),
        mechanic: Some(entry.id.clone()),
        pattern: None,
        limit: request.limit,
        cursor: None,
        mode: LeadMode::Full,
        page_bytes: request.page_bytes,
        legacy_paging: crate::tools::LegacyPaging::default(),
    };
    let first_count = leads_with_catalog(&first, limits, &catalog, sink, manager)?;
    session.with_entries(|entries| {
        check_free(entries, &entry.id)?;
        entries.push(entry.clone());
        Ok::<(), ErrorData>(())
    })??;
    Ok((entry.id, MechanicSource::Session, first_count))
}

/// Places a candidate where it will be listed: after the session entries.
fn insert_candidate(catalog: &mut Catalog, entry: MechanicEntry, query: tree_sitter::Query) {
    catalog.custom.push(Registered {
        entry,
        source: MechanicSource::Session,
        query,
    });
}

/// Removes the session entry `id`. A built-in is refused. Returns the removed
/// entry's id and its source.
pub(super) fn remove(
    request: &MechanicRequest,
    manager: &sync::SourceSyncManager,
) -> Result<(String, MechanicSource), ErrorData> {
    let id = required(request.id.as_ref(), "id", "remove")?;
    if is_builtin(&id) {
        return Err(ErrorData::invalid_params(
            format!("`{id}` is a built-in mechanic; built-ins can never be removed"),
            None,
        ));
    }
    let unknown = || {
        ErrorData::invalid_params(
            format!("no registered mechanic has id `{id}`"),
            error_codes::payload(ErrorCode::UnknownMechanic),
        )
    };
    let session = manager.session_mechanics();
    session.with_entries(|entries| {
        entries
            .iter()
            .any(|entry| entry.id == id)
            .then_some(())
            .ok_or_else(unknown)
    })??;
    // The cache goes first: if clearing it fails, the call fails with the
    // entry still registered, so "failed" never describes a removed entry.
    manager.mechanic_scan_cache().remove_id(&id)?;
    session.with_entries(|entries| {
        let position = entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(unknown)?;
        entries.remove(position);
        Ok::<(), ErrorData>(())
    })??;
    Ok((id, MechanicSource::Session))
}
