//! The mechanic catalogue one call sees: this server's session entries, the
//! project's persisted entries, and the built-ins — in that order — plus every
//! entry that could not be loaded, with its reason.
//!
//! The invariants live here: an entry's id is `[a-z0-9_]{1,64}`, never a
//! built-in's, and unique across the session and the project; its query
//! compiles against its language and names exactly one capture. `add` is
//! refused when any rule fails; a persisted entry that fails one on load is
//! listed in `invalid`, never dropped, and never shadows a built-in or a
//! session entry.

use super::leads_mechanics::MECHANICS;
use super::mechanic::{InvalidMechanic, MechanicListing, MechanicSource};
use crate::model::ResourceLimits;
use crate::project::{MechanicEntry, SessionMechanics};
use std::path::Path;
use tree_sitter::Query;

/// Longest entry id, in bytes.
pub(super) const MAX_ID_BYTES: usize = 64;
/// Longest note, in bytes.
pub(super) const MAX_NOTE_BYTES: usize = 1024;
/// Longest language name, in bytes.
pub(super) const MAX_LANGUAGE_BYTES: usize = 64;

/// A loaded entry with its compiled query.
pub(super) struct Registered {
    pub(super) entry: MechanicEntry,
    pub(super) source: MechanicSource,
    pub(super) query: Query,
}

/// Everything registered for one call.
pub(super) struct Catalog {
    /// Session entries (oldest first), then persisted ones (file order).
    pub(super) custom: Vec<Registered>,
    /// Entries that could not be loaded.
    pub(super) invalid: Vec<InvalidMechanic>,
}

impl Catalog {
    /// Every loaded entry, session first, then project_config, then builtin.
    pub(super) fn listings(&self) -> Vec<MechanicListing> {
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
    pub(super) fn position(&self, id: &str) -> Option<usize> {
        self.custom
            .iter()
            .position(|registered| registered.entry.id == id)
    }
}

/// True for the id of a built-in mechanic.
pub(super) fn is_builtin(id: &str) -> bool {
    MECHANICS.iter().any(|mechanic| mechanic.id == id)
}

/// The field rules every entry obeys, checked before anything is compiled.
fn check_shape(entry: &MechanicEntry, limits: &ResourceLimits) -> Result<(), String> {
    let id_ok = !entry.id.is_empty()
        && entry.id.len() <= MAX_ID_BYTES
        && entry
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
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
    if entry.language.is_empty() || entry.language.len() > MAX_LANGUAGE_BYTES {
        return Err(format!(
            "language must be non-empty and at most {MAX_LANGUAGE_BYTES} bytes"
        ));
    }
    if crate::parse::is_prose_language_name(&entry.language) {
        return Err(format!(
            "language `{}` is prose: prose files are stored but never parsed, so no query can \
             run on them",
            entry.language
        ));
    }
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

/// Checks `entry`'s fields and compiles its query against the real grammar.
/// The error names the rule that failed, or carries the compiler's message.
pub(super) fn compile(entry: &MechanicEntry, limits: &ResourceLimits) -> Result<Query, String> {
    check_shape(entry, limits)?;
    if !crate::parse::language_available(&entry.language) {
        return Err(format!(
            "unknown language `{}`: the language pack has no grammar by that name",
            entry.language
        ));
    }
    let grammar = tree_sitter_language_pack::get_language(&entry.language)
        .map_err(|error| format!("grammar `{}` could not be loaded: {error}", entry.language))?;
    let query = Query::new(&grammar, &entry.query)
        .map_err(|error| format!("query does not compile: {error}"))?;
    let captures = query.capture_names().len();
    if captures != 1 {
        return Err(format!(
            "query must name exactly one capture; it names {captures}"
        ));
    }
    Ok(query)
}

/// Loads the catalogue for `project_root`: compiles every session entry and
/// every persisted one, and reports what fails instead of dropping it.
pub(super) fn load(
    project_root: &Path,
    session: &SessionMechanics,
    limits: &ResourceLimits,
) -> Catalog {
    let mut catalog = Catalog {
        custom: Vec::new(),
        invalid: Vec::new(),
    };
    for (index, entry) in session.entries().into_iter().enumerate() {
        let checked = collision(&catalog, &entry.id).and_then(|()| compile(&entry, limits));
        match checked {
            Ok(query) => catalog.custom.push(Registered {
                entry,
                source: MechanicSource::Session,
                query,
            }),
            Err(error) => catalog.invalid.push(InvalidMechanic {
                source: MechanicSource::Session,
                index: Some(index),
                id: Some(entry.id),
                error,
            }),
        }
    }
    let persisted = crate::project::load_persisted(project_root);
    let mut invalid: Vec<InvalidMechanic> = persisted
        .invalid
        .into_iter()
        .map(|failed| InvalidMechanic {
            source: MechanicSource::ProjectConfig,
            index: failed.index,
            id: failed.id,
            error: failed.error,
        })
        .collect();
    for (index, entry) in persisted.entries {
        let checked = collision(&catalog, &entry.id).and_then(|()| compile(&entry, limits));
        match checked {
            Ok(query) => catalog.custom.push(Registered {
                entry,
                source: MechanicSource::ProjectConfig,
                query,
            }),
            Err(error) => invalid.push(InvalidMechanic {
                source: MechanicSource::ProjectConfig,
                index: Some(index),
                id: Some(entry.id),
                error,
            }),
        }
    }
    invalid.sort_by_key(|failed| failed.index);
    catalog.invalid.extend(invalid);
    catalog
}

/// Why `id` cannot join the catalogue, if it cannot.
fn collision(catalog: &Catalog, id: &str) -> Result<(), String> {
    if is_builtin(id) {
        return Err(format!(
            "id `{id}` is a built-in mechanic's; a registered mechanic can never replace one"
        ));
    }
    match catalog
        .position(id)
        .and_then(|index| catalog.custom.get(index))
    {
        Some(existing) if existing.source == MechanicSource::Session => {
            Err(format!("id `{id}` is already registered for this session"))
        }
        Some(_) => Err(format!(
            "id `{id}` is already used by an earlier persisted mechanic"
        )),
        None => Ok(()),
    }
}
