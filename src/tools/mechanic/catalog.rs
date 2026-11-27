//! The mechanic catalogue one call sees: this project's session entries and
//! the built-ins, in that order, plus entry validation and Tree-sitter query
//! compilation for `mechanic add`.

use super::{MechanicListing, MechanicSource};
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::ResourceLimits;
use crate::project::{MechanicEntry, SessionMechanics};
use crate::tools::grammar::{
    check_non_alias_non_prose_language, load_grammar, map_grammar_load_error,
};
use crate::tools::leads::mechanics::MECHANICS;
use crate::util::LockPoisoned;
use rmcp::ErrorData;
use std::path::Path;
use std::sync::Arc;
use tree_sitter::Query;

/// Longest entry id, in bytes.
pub(super) const MAX_ID_BYTES: usize = 64;
/// Longest note, in bytes.
pub(super) const MAX_NOTE_BYTES: usize = 1024;
/// Longest language name, in bytes.
pub(super) const MAX_LANGUAGE_BYTES: usize = 64;

/// A loaded entry with its pre-compiled Tree-sitter query.
pub(in crate::tools) struct Registered {
    pub(in crate::tools) entry: MechanicEntry,
    pub(in crate::tools) source: MechanicSource,
    pub(in crate::tools) query: Arc<Query>,
}

/// Everything registered for one project on one call.
pub(in crate::tools) struct Catalog {
    /// Session entries for this project, oldest first.
    pub(in crate::tools) custom: Vec<Registered>,
}

impl Catalog {
    /// Every loaded entry, session first, then builtin.
    pub(in crate::tools) fn listings(&self) -> Vec<MechanicListing> {
        let custom = self.custom.iter().map(|registered| MechanicListing {
            id: registered.entry.id.clone(),
            source: registered.source,
            note: Some(registered.entry.note.clone()),
            language: Some(registered.entry.language.clone()),
            query: Some(registered.entry.query.clone()),
        });
        let builtin = MECHANICS.iter().map(|mechanic| MechanicListing {
            id: mechanic.id.to_owned(),
            source: MechanicSource::Builtin,
            note: None,
            language: None,
            query: None,
        });
        custom.chain(builtin).collect()
    }

    /// Position of a loaded entry in `custom`.
    pub(in crate::tools) fn position(&self, id: &str) -> Option<usize> {
        self.custom
            .iter()
            .position(|registered| registered.entry.id == id)
    }
}

/// True for the id of a built-in mechanic.
pub(super) fn is_builtin(id: &str) -> bool {
    MECHANICS.iter().any(|mechanic| mechanic.id == id)
}

/// True when every byte of `text` is one of `a-z`, `0-9` or `_`.
pub(super) fn is_lower_snake(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Validates field rules on `entry` before loading a grammar or compiling a query.
fn check_shape(entry: &MechanicEntry, limits: &ResourceLimits) -> Result<(), String> {
    let id_ok = !entry.id.is_empty() && entry.id.len() <= MAX_ID_BYTES && is_lower_snake(&entry.id);
    if !id_ok {
        return Err(format!(
            "id must be 1 to {MAX_ID_BYTES} characters of a-z, 0-9 and _"
        ));
    }
    if entry.note.trim().is_empty() || entry.note.len() > MAX_NOTE_BYTES {
        return Err(format!(
            "note must be non-empty and at most {MAX_NOTE_BYTES} bytes"
        ));
    }
    if entry.language.is_empty()
        || entry.language.len() > MAX_LANGUAGE_BYTES
        || !is_lower_snake(&entry.language)
    {
        return Err(format!(
            "language must be 1 to {MAX_LANGUAGE_BYTES} characters of a-z, 0-9 and _: the \
             lowercase pack name the index stores for a file (python, rust, javascript, tsx, \
             csharp), compared exactly"
        ));
    }
    check_non_alias_non_prose_language(&entry.language)?;
    if entry.query.trim().is_empty() {
        return Err("query must not be empty".to_owned());
    }
    if entry.query.len() > limits.max_structure_query_bytes {
        return Err(format!(
            "query is {} bytes; the limit is {} (max_structure_query_bytes)",
            entry.query.len(),
            limits.max_structure_query_bytes
        ));
    }
    Ok(())
}

/// Validates `entry` against `limits` and compiles its Tree-sitter [`Query`].
pub(super) fn compile(
    entry: &MechanicEntry,
    limits: &ResourceLimits,
) -> Result<Arc<Query>, ErrorData> {
    if let Err(reason) = check_shape(entry, limits) {
        return Err(invalid_params_with_code(
            format!("mechanic not added: {reason}"),
            ErrorCode::InvalidInput,
        ));
    }
    if !crate::parse::language_available(&entry.language) {
        return Err(invalid_params_with_code(
            format!(
                "mechanic not added: unknown language `{}`: the language pack has no grammar by \
                 that name",
                entry.language
            ),
            ErrorCode::InvalidInput,
        ));
    }
    let grammar = load_grammar(&entry.language).map_err(|error| {
        map_grammar_load_error(&entry.language, Some("mechanic not added"), error)
    })?;
    let query = Query::new(&grammar, &entry.query).map_err(|error| {
        invalid_params_with_code(
            format!("mechanic not added: query does not compile: {error}"),
            ErrorCode::InvalidTreeSitterQuery,
        )
    })?;
    let captures = query.capture_names().len();
    if captures != 1 {
        return Err(invalid_params_with_code(
            format!("mechanic not added: query must name exactly one capture; it names {captures}"),
            ErrorCode::InvalidTreeSitterQuery,
        ));
    }
    Ok(Arc::new(query))
}

/// Loads `project_root`'s pre-compiled session mechanics into a [`Catalog`].
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the session-mechanics mutex was poisoned.
pub(in crate::tools) fn load(
    session: &SessionMechanics,
    project_root: &Path,
) -> Result<Catalog, LockPoisoned> {
    let custom = session
        .entries(project_root)?
        .into_iter()
        .map(|compiled| Registered {
            entry: compiled.entry,
            source: MechanicSource::Session,
            query: compiled.compiled,
        })
        .collect();
    Ok(Catalog { custom })
}
