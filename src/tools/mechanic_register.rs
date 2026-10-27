// slugaudit-line-exception: approved-by=slugthug; reason=every change to the mechanic registry (add, remove by id, remove by index) sits beside the check that guards it and the one error mapping they share
//! `mechanic add` and `mechanic remove`: the registration rules and the
//! writes they guard. Split from `mechanic.rs` (the tool contract) because
//! this is the only code that changes the registry — the session list and
//! the persisted section — so every check that must hold before a change
//! (built-in ids, collisions, caps) sits beside the change it guards.

use super::leads::{LeadMode, LeadsRequest, LeadsResponse, leads_with_catalog};
use super::mechanic::{MechanicRequest, MechanicSource};
use super::mechanic_catalog::{self, Catalog, Registered, is_builtin};
use crate::error_codes::{self, ErrorCode};
use crate::progress::ProgressSink;
use crate::project::{
    ConfigWriteError, MAX_PERSISTED_MECHANICS, MAX_SESSION_MECHANICS, MechanicEntry,
    MechanicsWriteError,
};
use crate::sync;
use rmcp::ErrorData;
use std::path::Path;

fn required(value: Option<&String>, name: &str, action: &str) -> Result<String, ErrorData> {
    value
        .cloned()
        .ok_or_else(|| ErrorData::invalid_params(format!("`{action}` needs `{name}`"), None))
}

fn refused(reason: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(format!("mechanic not added: {reason}"), None)
}

/// Refuses `id` when it is a built-in's, already registered for this
/// session, or present in `config.json` (valid or not), or when the list it
/// would join is full.
fn check_free(
    root: &Path,
    session: &[MechanicEntry],
    id: &str,
    persist: bool,
) -> Result<(), ErrorData> {
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
    let persisted = crate::project::load_persisted(root);
    let taken = persisted.entries.iter().any(|(_, entry)| entry.id == id)
        || persisted
            .invalid
            .iter()
            .any(|failed| failed.id.as_deref() == Some(id));
    if taken {
        return Err(refused(format!(
            "`{id}` is already persisted in config.json"
        )));
    }
    let stored = persisted.entries.len()
        + persisted
            .invalid
            .iter()
            .filter(|failed| failed.index.is_some())
            .count();
    if persist && stored >= MAX_PERSISTED_MECHANICS {
        return Err(refused(format!(
            "config.json already holds {MAX_PERSISTED_MECHANICS} persisted mechanics, the cap"
        )));
    }
    if !persist && session.len() >= MAX_SESSION_MECHANICS {
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
    root: &Path,
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
    session.with_entries(|entries| check_free(root, entries, &entry.id, request.persist))?;
    let source = if request.persist {
        MechanicSource::ProjectConfig
    } else {
        MechanicSource::Session
    };
    let mut catalog = mechanic_catalog::load(root, session, limits);
    insert_candidate(&mut catalog, entry.clone(), source, query);
    let first = LeadsRequest {
        path: path.to_owned(),
        mechanic: Some(entry.id.clone()),
        pattern: None,
        limit: request.limit,
        offset: None,
        expected_revision: None,
        mode: LeadMode::Full,
        page_bytes: request.page_bytes,
    };
    let first_count = leads_with_catalog(&first, limits, &catalog, sink, manager)?;
    session.with_entries(|entries| {
        check_free(root, entries, &entry.id, request.persist)?;
        if request.persist {
            crate::project::persist_entry(root, &entry).map_err(write_error)
        } else {
            entries.push(entry.clone());
            Ok(())
        }
    })?;
    Ok((entry.id, source, first_count))
}

/// Places a candidate where it will be listed: after the session entries for
/// a session entry, at the end for a persisted one.
fn insert_candidate(
    catalog: &mut Catalog,
    entry: MechanicEntry,
    source: MechanicSource,
    query: tree_sitter::Query,
) {
    let at = match source {
        MechanicSource::Session => catalog
            .custom
            .iter()
            .take_while(|registered| registered.source == MechanicSource::Session)
            .count(),
        MechanicSource::ProjectConfig | MechanicSource::Builtin => catalog.custom.len(),
    };
    catalog.custom.insert(
        at,
        Registered {
            entry,
            source,
            query,
        },
    );
}

/// Removes the session entry `id`, or else the persisted one, or the invalid
/// persisted entry at `index`. A built-in is refused. Returns the removed
/// entry's id (none for an index whose entry has no string id).
pub(super) fn remove(
    request: &MechanicRequest,
    root: &Path,
    limits: &crate::model::ResourceLimits,
    manager: &sync::SourceSyncManager,
) -> Result<(Option<String>, MechanicSource), ErrorData> {
    if let Some(index) = request.index {
        return remove_invalid_at(root, index, limits, manager)
            .map(|id| (id, MechanicSource::ProjectConfig));
    }
    let id = required(request.id.as_ref(), "id", "remove")?;
    if is_builtin(&id) {
        return Err(ErrorData::invalid_params(
            format!(
                "`{id}` is a built-in mechanic; built-ins can never be removed (a persisted \
                 entry using this id is listed in `invalid`: remove it by its `index`)"
            ),
            None,
        ));
    }
    let source = manager.session_mechanics().with_entries(|entries| {
        if let Some(position) = entries.iter().position(|entry| entry.id == id) {
            entries.remove(position);
            return Ok(MechanicSource::Session);
        }
        match crate::project::remove_persisted(root, &id) {
            Ok(()) => Ok(MechanicSource::ProjectConfig),
            Err(MechanicsWriteError::NotFound(_)) => Err(ErrorData::invalid_params(
                format!("no session or persisted mechanic has id `{id}`"),
                error_codes::payload(ErrorCode::UnknownMechanic),
            )),
            Err(error) => Err(write_error(error)),
        }
    })?;
    manager.mechanic_scan_cache().remove_id(&id);
    Ok((Some(id), source))
}

/// Removes the persisted entry at `index` if, judged under the config lock
/// with this server's session entries, it is still listed invalid: the same
/// catalogue rules `report` and `list` apply, over the file as it is now, so
/// an index that moved (another process removed an entry) can never remove a
/// valid entry.
fn remove_invalid_at(
    root: &Path,
    index: usize,
    limits: &crate::model::ResourceLimits,
    manager: &sync::SourceSyncManager,
) -> Result<Option<String>, ErrorData> {
    manager.session_mechanics().with_entries(|session| {
        let mut removed = None;
        crate::project::remove_persisted_at(root, index, |persisted| {
            let catalog = mechanic_catalog::load_from(session, persisted.clone(), limits);
            let invalid = catalog.invalid.into_iter().find(|failed| {
                failed.source == MechanicSource::ProjectConfig && failed.index == Some(index)
            });
            let found = invalid.is_some();
            removed = invalid.and_then(|failed| failed.id);
            found
        })
        .map_err(write_error)?;
        Ok(removed)
    })
}

/// Maps a failed `config.json` update to the tool error, in the style of
/// `project_control`'s config write: a file or request the caller must fix (a
/// symlink, malformed JSON, a section this server cannot extend, a full
/// list, a duplicate id, an index that is not an invalid entry) is
/// `invalid_params`; an I/O failure, a lock held past the bounded wait, or a
/// directory sync that failed after the replace is operational.
fn write_error(error: MechanicsWriteError) -> ErrorData {
    let operational = matches!(
        &error,
        MechanicsWriteError::Config(
            ConfigWriteError::Io { .. }
                | ConfigWriteError::Locked { .. }
                | ConfigWriteError::NotDurable { .. }
        )
    );
    if operational {
        super::context::internal_error("writing the mechanics config", error)
    } else {
        ErrorData::invalid_params(format!("mechanics config not changed: {error}"), None)
    }
}
