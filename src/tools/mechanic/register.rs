//! `mechanic add` and `mechanic remove`: registration rules, trial-scan
//! completeness gate, and per-project session-list updates.

use super::catalog::{self as mechanic_catalog, Catalog, Registered, is_builtin};
use super::{MechanicRequest, MechanicSource};
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::progress::ProgressSink;
use crate::project::{CompiledMechanic, MAX_SESSION_MECHANICS, MechanicEntry};
use crate::sync;
use crate::tools::leads::{LeadMode, LeadsRequest, LeadsResponse, leads_with_catalog};
use crate::tools::page::{Completeness, PageLimit};
use rmcp::ErrorData;
use std::path::Path;
use std::sync::Arc;

fn required(value: Option<&String>, name: &str, action: &str) -> Result<String, ErrorData> {
    value.cloned().ok_or_else(|| {
        invalid_params_with_code(
            format!("`{action}` needs `{name}`"),
            ErrorCode::InvalidInput,
        )
    })
}

fn refused(reason: impl std::fmt::Display) -> ErrorData {
    invalid_params_with_code(
        format!("mechanic not added: {reason}"),
        ErrorCode::InvalidInput,
    )
}

/// Refuses `id` when it is a built-in's or already registered for this
/// project's session, or when the project's session list is full.
fn check_free(session: &[CompiledMechanic], id: &str) -> Result<(), ErrorData> {
    if is_builtin(id) {
        return Err(refused(format!(
            "`{id}` is a built-in mechanic's id; a registered mechanic can never remove, rename \
             or override a built-in"
        )));
    }
    if session.iter().any(|compiled| compiled.entry.id == id) {
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

/// Compiles and checks `entry`, runs a trial `leads` scan on `project_root`,
/// refuses registration if the trial scan is `Partial`, and otherwise commits
/// the pre-compiled mechanic to `project_root`'s session list.
pub(super) fn add(
    request: &MechanicRequest,
    path: &str,
    project_root: &Path,
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
    let compiled = mechanic_catalog::compile(&entry, limits)?;
    let session = manager.session_mechanics();
    session.with_entries(project_root, |entries| check_free(entries, &entry.id))??;
    let mut catalog = mechanic_catalog::load(session, project_root)?;
    insert_candidate(&mut catalog, entry.clone(), Arc::clone(&compiled));
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
    if let Some(Completeness::Partial { limited_by, .. }) = &first_count.page.completeness {
        let reason = match limited_by {
            PageLimit::QueryMatchLimit => {
                "exceeded the Tree-sitter QueryCursor match limit (65,536 active states)"
            }
            PageLimit::TimeBudget => "exceeded max_structure_execution_time",
            PageLimit::None
            | PageLimit::PageBytes
            | PageLimit::Limit
            | PageLimit::MatchCap
            | PageLimit::ScanWindow
            | PageLimit::Ceiling => "stopped before every file was scanned",
        };
        return Err(invalid_params_with_code(
            format!(
                "mechanic not added: trial scan for `{}` was incomplete ({reason}); narrow the \
                 Tree-sitter query pattern before registering it",
                entry.id
            ),
            ErrorCode::MechanicPatternTooBroad,
        ));
    }
    let id = entry.id.clone();
    session.with_entries(project_root, |entries| {
        check_free(entries, &entry.id)?;
        entries.push(CompiledMechanic { entry, compiled });
        Ok::<(), ErrorData>(())
    })??;
    Ok((id, MechanicSource::Session, first_count))
}

/// Places a candidate where it will be listed: after the session entries.
fn insert_candidate(catalog: &mut Catalog, entry: MechanicEntry, query: Arc<tree_sitter::Query>) {
    catalog.custom.push(Registered {
        entry,
        source: MechanicSource::Session,
        query,
    });
}

/// Removes the session entry `id` for `project_root`. A built-in is refused.
pub(super) fn remove(
    request: &MechanicRequest,
    project_root: &Path,
    manager: &sync::SourceSyncManager,
) -> Result<(String, MechanicSource), ErrorData> {
    let id = required(request.id.as_ref(), "id", "remove")?;
    if is_builtin(&id) {
        return Err(invalid_params_with_code(
            format!("`{id}` is a built-in mechanic; built-ins can never be removed"),
            ErrorCode::InvalidInput,
        ));
    }
    let unknown = || {
        invalid_params_with_code(
            format!("no registered mechanic has id `{id}`"),
            ErrorCode::UnknownMechanic,
        )
    };
    manager
        .session_mechanics()
        .with_entries(project_root, |entries| {
            let position = entries
                .iter()
                .position(|compiled| compiled.entry.id == id)
                .ok_or_else(unknown)?;
            entries.remove(position);
            Ok::<(), ErrorData>(())
        })??;
    Ok((id, MechanicSource::Session))
}
