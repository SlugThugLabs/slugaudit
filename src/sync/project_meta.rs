//! Singleton `project` table metadata SQL helpers and post-sync revision /
//! instance-token lookups.

use super::revision::current_revision_id;
use crate::store;
use rmcp::ErrorData;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

const PROJECT_CONTRACT_VERSION: i64 = 1;
const PROJECT_SCHEMA_VERSION: i64 = 1;
const PROJECT_META_SELECT_SQL: &str =
    "SELECT contract_version, schema_version, root_path FROM project WHERE id = 1";

/// Reads the per-database instance token (`project.project_id`) from the
/// singleton `project` metadata row, or `None` if no metadata row exists yet.
pub(crate) fn project_instance_id(
    connection: &Connection,
) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row("SELECT project_id FROM project WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()
}

/// Reads the database instance token after a sync, shaping any SQLite error
/// with `database_path`.
pub(super) fn instance_id_after_sync(
    connection: &Connection,
    database_path: &Path,
) -> Result<Option<String>, ErrorData> {
    project_instance_id(connection).map_err(|error| {
        ErrorData::internal_error(
            format!(
                "reading the project instance id of {}: {error}",
                database_path.display()
            ),
            None,
        )
    })
}

/// Reads the current revision id after a sync, returning an internal error
/// naming `database_path` if missing or unreadable.
pub(super) fn revision_after_sync(
    connection: &Connection,
    database_path: &Path,
) -> Result<String, ErrorData> {
    current_revision_id(connection)
        .map_err(|error| {
            ErrorData::internal_error(
                format!(
                    "reading the current revision of {}: {error}",
                    database_path.display()
                ),
                None,
            )
        })?
        .ok_or_else(|| {
            ErrorData::internal_error(
                format!(
                    "no current revision found after sync in {} — this is unexpected; \
                     try disabling and re-enabling the project",
                    database_path.display()
                ),
                None,
            )
        })
}

/// Errors from verifying or creating a project's singleton metadata row.
#[derive(Debug)]
pub(super) enum ProjectMetaError {
    /// Stored contract or schema version is unsupported; discard and rebuild.
    IncompatibleVersion {
        contract_version: i64,
        schema_version: i64,
    },
    /// Database was previously built for a different canonical project root.
    RootMismatch { stored_root: String },
    /// SQLite reported corruption while reading or recording metadata.
    Corrupt(rusqlite::Error),
    /// Non-recoverable metadata error surfaced directly.
    Other(ErrorData),
}

impl std::fmt::Display for ProjectMetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompatibleVersion {
                contract_version,
                schema_version,
            } => write!(
                f,
                "database has an unsupported contract/schema version \
                 ({contract_version}/{schema_version})"
            ),
            Self::RootMismatch { stored_root } => write!(
                f,
                "database was built for a different project root ({stored_root})"
            ),
            Self::Corrupt(error) => write!(f, "project database is corrupt: {error}"),
            Self::Other(error) => write!(f, "{}", error.message),
        }
    }
}

impl From<ProjectMetaError> for ErrorData {
    fn from(value: ProjectMetaError) -> Self {
        match value {
            ProjectMetaError::Other(error) => error,
            ProjectMetaError::Corrupt(error) => ErrorData::internal_error(
                format!("project database is corrupt while verifying metadata: {error}"),
                None,
            ),
            ProjectMetaError::RootMismatch { stored_root } => ErrorData::invalid_params(
                format!(
                    "database at this location was built for a different project root \
                     ({stored_root}); discard the .planning/slugaudit directory (or \
                     re-enable the project) so it is rebuilt from source"
                ),
                None,
            ),
            ProjectMetaError::IncompatibleVersion {
                contract_version,
                schema_version,
            } => ErrorData::invalid_params(
                format!(
                    "database at this location was built by an unsupported SlugAudit \
                     contract/schema version ({contract_version}/{schema_version}); discard \
                     the .planning/slugaudit directory (or re-enable the project) so it is \
                     rebuilt from source"
                ),
                None,
            ),
        }
    }
}

fn verify_project_row(
    contract_version: i64,
    schema_version: i64,
    stored_root: String,
    expected_root: &str,
) -> Result<(), ProjectMetaError> {
    if contract_version != PROJECT_CONTRACT_VERSION || schema_version != PROJECT_SCHEMA_VERSION {
        return Err(ProjectMetaError::IncompatibleVersion {
            contract_version,
            schema_version,
        });
    }
    if stored_root != expected_root {
        return Err(ProjectMetaError::RootMismatch { stored_root });
    }
    Ok(())
}

fn map_metadata_sql_error(error: rusqlite::Error, context: &str) -> ProjectMetaError {
    if store::is_rusqlite_corruption(&error) {
        return ProjectMetaError::Corrupt(error);
    }
    ProjectMetaError::Other(ErrorData::internal_error(
        format!("{context}: {error}"),
        None,
    ))
}

/// Inserts the singleton `project` row on first sync if absent and verifies
/// `contract_version`, `schema_version`, and `root_path` against `root`.
pub(super) fn ensure_project_row(
    connection: &mut Connection,
    root: &Path,
) -> Result<(), ProjectMetaError> {
    let expected_root = root.to_string_lossy();
    match connection
        .query_row(PROJECT_META_SELECT_SQL, [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()
    {
        Ok(Some((contract_version, schema_version, stored_root))) => verify_project_row(
            contract_version,
            schema_version,
            stored_root,
            expected_root.as_ref(),
        ),
        Ok(None) => {
            let created_at =
                crate::util::now_unix().map_err(|error| ProjectMetaError::Other(error.into()))?;
            let instance_id = uuid::Uuid::new_v4().to_string();
            connection
                .execute(
                    "INSERT OR IGNORE INTO project (\
                        id, project_id, root_path, contract_version, schema_version, created_at_unix\
                     ) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        instance_id,
                        expected_root.as_ref(),
                        PROJECT_CONTRACT_VERSION,
                        PROJECT_SCHEMA_VERSION,
                        created_at
                    ],
                )
                .map_err(|error| map_metadata_sql_error(error, "recording project metadata"))?;
            let (contract_version, schema_version, stored_root) = connection
                .query_row(PROJECT_META_SELECT_SQL, [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(|error| map_metadata_sql_error(error, "reading project metadata"))?;
            verify_project_row(
                contract_version,
                schema_version,
                stored_root,
                expected_root.as_ref(),
            )
        }
        Err(error) => Err(map_metadata_sql_error(error, "reading project metadata")),
    }
}

#[cfg(test)]
#[path = "manager_meta_tests.rs"]
mod tests;
