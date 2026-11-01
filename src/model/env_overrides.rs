//! The `SLUGAUDIT_*` environment as data.
//!
//! The environment is read and validated once ([`env_overrides`]) and applied
//! to the limits of every call, so a tool call never re-reads (or fails on)
//! variables that were valid at startup, and one `SLUGAUDIT_MAX_FILE_BYTES`
//! clamp warning is logged once instead of on every call.

use super::env_error::EnvConfigError;
use super::limit_overrides::nonzero_override;
use super::limits::{ResourceLimits, SQLITE_MAX_VALUE_BYTES};
use super::profile::{AuditProfile, profile_from_env, result_store_budget};
use std::sync::OnceLock;
use std::time::Duration;

/// Every limit override the environment sets; `None` is "unset".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EnvOverrides {
    /// `SLUGAUDIT_PROFILE`.
    pub(crate) profile: Option<AuditProfile>,
    pub(crate) max_file_bytes: Option<u64>,
    pub(crate) max_parse_bytes: Option<u64>,
    pub(crate) max_parse_wall_clock_ms: Option<u64>,
    pub(crate) max_total_import_bytes: Option<u64>,
    pub(crate) max_query_response_bytes: Option<usize>,
    pub(crate) default_page_bytes: Option<usize>,
    pub(crate) max_query_sql_bytes: Option<usize>,
    pub(crate) max_query_vm_steps: Option<u32>,
    pub(crate) max_query_wall_clock_secs: Option<u64>,
    pub(crate) max_query_value_bytes: Option<usize>,
    pub(crate) max_structure_query_bytes: Option<usize>,
    pub(crate) max_structure_matches: Option<usize>,
    pub(crate) max_structure_execution_time_secs: Option<u64>,
    pub(crate) max_sync_wall_clock_secs: Option<u64>,
    pub(crate) max_result_store_bytes: Option<u64>,
    pub(crate) result_active_grace_secs: Option<u64>,
}

static ENV: OnceLock<EnvOverrides> = OnceLock::new();

/// The process's environment overrides: read and validated on first use,
/// then fixed. An invalid environment installs nothing, so every later call
/// validates again instead of finding a fallback.
///
/// # Errors
///
/// Returns [`EnvConfigError`] naming the offending variable.
pub(crate) fn env_overrides() -> Result<&'static EnvOverrides, EnvConfigError> {
    if let Some(env) = ENV.get() {
        return Ok(env);
    }
    let env = EnvOverrides::read()?;
    Ok(ENV.get_or_init(|| env))
}

impl EnvOverrides {
    /// Reads every `SLUGAUDIT_*` limit variable. Unset variables stay `None`,
    /// so a typo in a variable *name* is harmless; a variable that is set but
    /// cannot be read or parsed (or is `0`) fails with an error naming it,
    /// never its value.
    ///
    /// # Errors
    ///
    /// Returns [`EnvConfigError`] for a set-but-invalid variable.
    pub(crate) fn read() -> Result<Self, EnvConfigError> {
        Ok(Self {
            profile: profile_from_env()?,
            max_file_bytes: nonzero_override::<u64>("SLUGAUDIT_MAX_FILE_BYTES")?
                .map(|bytes| clamp_sqlite_value("SLUGAUDIT_MAX_FILE_BYTES", bytes)),
            max_parse_bytes: nonzero_override("SLUGAUDIT_MAX_PARSE_BYTES")?,
            max_parse_wall_clock_ms: nonzero_override("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS")?,
            max_total_import_bytes: nonzero_override("SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES")?,
            max_query_response_bytes: nonzero_override("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES")?,
            default_page_bytes: nonzero_override("SLUGAUDIT_DEFAULT_PAGE_BYTES")?,
            max_query_sql_bytes: nonzero_override("SLUGAUDIT_MAX_QUERY_SQL_BYTES")?,
            max_query_vm_steps: nonzero_override("SLUGAUDIT_MAX_QUERY_VM_STEPS")?,
            max_query_wall_clock_secs: nonzero_override("SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS")?,
            max_query_value_bytes: nonzero_override::<usize>("SLUGAUDIT_MAX_QUERY_VALUE_BYTES")?
                .map(|bytes| {
                    clamp_sqlite_value("SLUGAUDIT_MAX_QUERY_VALUE_BYTES", bytes as u64) as usize
                }),
            max_structure_query_bytes: nonzero_override("SLUGAUDIT_MAX_STRUCTURE_QUERY_BYTES")?,
            max_structure_matches: nonzero_override("SLUGAUDIT_MAX_STRUCTURE_MATCHES")?,
            max_structure_execution_time_secs: nonzero_override(
                "SLUGAUDIT_MAX_STRUCTURE_EXECUTION_TIME_SECS",
            )?,
            max_sync_wall_clock_secs: nonzero_override("SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS")?,
            max_result_store_bytes: nonzero_override("SLUGAUDIT_MAX_RESULT_STORE_BYTES")?,
            result_active_grace_secs: nonzero_override("SLUGAUDIT_RESULT_ACTIVE_GRACE_SECS")?,
        })
    }

    /// Applies every override to `limits`.
    ///
    /// The result store budget follows the effective response ceiling (after
    /// its own override) unless something already set it:
    /// `result_store_pinned` is true when the global file set it, and then
    /// only its own variable replaces it.
    pub(crate) fn apply(&self, limits: &mut ResourceLimits, result_store_pinned: bool) {
        set(&mut limits.max_file_bytes, self.max_file_bytes);
        set(&mut limits.max_parse_bytes, self.max_parse_bytes);
        set(
            &mut limits.max_parse_wall_clock_ms,
            self.max_parse_wall_clock_ms,
        );
        set(
            &mut limits.max_total_import_bytes,
            self.max_total_import_bytes,
        );
        set(
            &mut limits.max_query_response_bytes,
            self.max_query_response_bytes,
        );
        set(&mut limits.default_page_bytes, self.default_page_bytes);
        set(&mut limits.max_query_sql_bytes, self.max_query_sql_bytes);
        set(&mut limits.max_query_vm_steps, self.max_query_vm_steps);
        set(
            &mut limits.max_query_wall_clock,
            self.max_query_wall_clock_secs.map(Duration::from_secs),
        );
        set(
            &mut limits.max_query_value_bytes,
            self.max_query_value_bytes,
        );
        set(
            &mut limits.max_structure_query_bytes,
            self.max_structure_query_bytes,
        );
        set(
            &mut limits.max_structure_matches,
            self.max_structure_matches,
        );
        set(
            &mut limits.max_structure_execution_time,
            self.max_structure_execution_time_secs
                .map(Duration::from_secs),
        );
        set(
            &mut limits.max_sync_wall_clock,
            self.max_sync_wall_clock_secs.map(Duration::from_secs),
        );
        if !result_store_pinned {
            limits.max_result_store_bytes = result_store_budget(limits.max_query_response_bytes);
        }
        set(
            &mut limits.max_result_store_bytes,
            self.max_result_store_bytes,
        );
        set(
            &mut limits.result_active_grace_secs,
            self.result_active_grace_secs,
        );
        limits.clamp_parse_to_file_bytes();
    }
}

/// Replaces `target` when the environment set a value.
fn set<T>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

/// Clamps one single-value limit to what the bundled SQLite can store.
///
/// An override above the cap is unusable: the value would be rejected at
/// insert/read. Clamp it and say so rather than letting a per-file skip turn
/// into a failed publish.
fn clamp_sqlite_value(name: &str, value: u64) -> u64 {
    if value > SQLITE_MAX_VALUE_BYTES {
        tracing::warn!(
            variable = name,
            requested = value,
            cap = SQLITE_MAX_VALUE_BYTES,
            "limit exceeds SQLite's maximum storable value; clamping",
        );
        SQLITE_MAX_VALUE_BYTES
    } else {
        value
    }
}
