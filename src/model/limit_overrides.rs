// slugaudit-line-exception: approved-by=split-judge; reason=single unified limit-override contract: 17-field LimitOverrides struct, SLUGAUDIT_* env parser, zero-check, SQLite value clamping, warn-once ledger, and layer application
//! Unified limit-override table (`LimitOverrides`), `SLUGAUDIT_*` environment
//! parsing, zero-value rejection, and warn-once SQLite/parse-cap clamping.

use super::limits::{ResourceLimits, SQLITE_MAX_VALUE_BYTES};
use super::profile::{AuditProfile, profile_from_env, result_store_budget};
use super::system_ram::MemoryDetectError;
use crate::util::{LockPoisoned, lock};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Mutex;
#[cfg(not(test))]
use std::sync::OnceLock;
use std::time::Duration;

/// A `SLUGAUDIT_*` or logging environment variable that was set to an unusable
/// value, or startup limit state that could not be derived.
///
/// Messages report the variable name and expected format without echoing raw
/// `OsString` bytes (`VarError::NotUnicode`'s `Display` prints lossy bytes
/// instead of a clean type hint).
#[derive(Debug, thiserror::Error)]
pub enum EnvConfigError {
    /// The variable is set but its value is not valid Unicode.
    #[error("{variable} is set but is not valid Unicode; unset it or set {expected}")]
    NotUnicode {
        /// Name of the variable.
        variable: String,
        /// Description of what is valid.
        expected: String,
    },
    /// The variable is set but its value cannot be parsed.
    #[error("{variable} is set but is not valid; expected {expected}")]
    Invalid {
        /// Name of the variable.
        variable: String,
        /// Description of what is valid.
        expected: String,
    },
    /// The variable is set to `0`, which is not a usable resource ceiling.
    #[error(
        "{variable} is set to 0, which is not a usable limit; set a positive value or unset it"
    )]
    Zero {
        /// Name of the variable.
        variable: String,
    },
    /// The `adaptive` profile requires host memory sizing that failed.
    #[error(
        "the adaptive profile needs the host memory size, which could not be detected ({source}); set SLUGAUDIT_PROFILE to `lean` or `audit`"
    )]
    MemoryUndetectable {
        /// Why memory detection failed.
        #[source]
        source: MemoryDetectError,
    },
    /// The clamp-warning ledger mutex was poisoned by an earlier panic.
    #[error(transparent)]
    WarningLedger(#[from] LockPoisoned),
}

type Ledger = BTreeSet<(&'static str, u64)>;

#[cfg(not(test))]
static REPORTED: Mutex<Ledger> = Mutex::new(BTreeSet::new());

#[cfg(test)]
thread_local! {
    static REPORTED: Mutex<Ledger> = const { Mutex::new(BTreeSet::new()) };
}

#[cfg(not(test))]
fn with_ledger<T>(change: impl FnOnce(&mut Ledger) -> T) -> Result<T, LockPoisoned> {
    Ok(change(&mut *lock(&REPORTED, "limit_override_warnings")?))
}

#[cfg(test)]
fn with_ledger<T>(change: impl FnOnce(&mut Ledger) -> T) -> Result<T, LockPoisoned> {
    REPORTED.with(|ledger| Ok(change(&mut *lock(ledger, "limit_override_warnings")?)))
}

/// Resets the calling test thread's clamp-warning ledger.
#[cfg(test)]
pub(crate) fn fresh_ledger() -> Result<(), LockPoisoned> {
    with_ledger(BTreeSet::clear)
}

fn first_report(setting: &'static str, value: u64) -> Result<bool, LockPoisoned> {
    with_ledger(|reported| reported.insert((setting, value)))
}

/// Clamps a single-value byte limit to `SQLITE_MAX_VALUE_BYTES`, logging once
/// per `(setting, value)` pair when clamped.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the warning ledger mutex was poisoned.
pub(crate) fn clamp_sqlite_value(setting: &'static str, value: u64) -> Result<u64, LockPoisoned> {
    if value <= SQLITE_MAX_VALUE_BYTES {
        return Ok(value);
    }
    if first_report(setting, value)? {
        tracing::warn!(
            setting,
            requested = value,
            cap = SQLITE_MAX_VALUE_BYTES,
            "limit exceeds SQLite's maximum storable value; clamping",
        );
    }
    Ok(SQLITE_MAX_VALUE_BYTES)
}

/// Clamps a `usize` byte limit to `SQLITE_MAX_VALUE_BYTES` via [`clamp_sqlite_value`].
pub(crate) fn clamp_sqlite_usize(
    setting: &'static str,
    value_bytes: usize,
) -> Result<usize, LockPoisoned> {
    let clamped = match u64::try_from(value_bytes) {
        Ok(wide) => clamp_sqlite_value(setting, wide)?,
        Err(_) => clamp_sqlite_value(setting, u64::MAX)?,
    };
    Ok(usize::try_from(clamped).map_or(value_bytes, |narrowed| narrowed))
}

/// Labels which configuration source supplied a [`LimitOverrides`] layer when
/// emitting clamp warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverrideSource {
    /// Human defaults from `~/.slugthug/slugaudit/config.json`.
    GlobalFile,
    /// Operator `SLUGAUDIT_*` environment variables.
    Environment,
}

macro_rules! define_limit_overrides {
    ($( $(#[$doc:meta])+ $field:ident : $ty:ty ),+ $(,)?) => {
        /// Optional overrides for `profile` and all 16 resource limits, shared
        /// by `~/.slugthug/slugaudit/config.json` (`GlobalConfig`) and
        /// `SLUGAUDIT_*` (`EnvOverrides`).
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct LimitOverrides {
            /// Desired audit resource profile (`lean`, `audit`, or `adaptive`).
            #[serde(default)]
            pub profile: Option<AuditProfile>,
            $(
                $(#[$doc])+
                #[serde(default)]
                pub $field: Option<$ty>,
            )+
        }

        impl LimitOverrides {
            /// Returns the first field name whose configured limit is `0`, if any.
            #[must_use]
            pub(crate) fn first_zero_key(&self) -> Option<&'static str> {
                $(
                    if let Some(val) = self.$field
                        && val == <$ty>::default()
                    {
                        return Some(stringify!($field));
                    }
                )+
                None
            }
        }
    };
}

define_limit_overrides! {
    /// Maximum individual file size in bytes to index.
    max_file_bytes: u64,
    /// Largest file content in bytes a grammar is run on; larger files are
    /// stored as `ContentOnly` (`oversize_for_parse`).
    max_parse_bytes: u64,
    /// Wall-clock allowance in milliseconds for every parse of one file.
    max_parse_wall_clock_ms: u64,
    /// Maximum cumulative size in bytes across all imported files in one sync.
    max_total_import_bytes: u64,
    /// Maximum serialized JSON bytes returned by one `query` or `leads` call.
    max_query_response_bytes: usize,
    /// Default `query`/`leads` page size in serialized JSON bytes.
    default_page_bytes: usize,
    /// Maximum SQL text length accepted by `query` in UTF-8 bytes.
    max_query_sql_bytes: usize,
    /// Soft cap on SQLite virtual-machine steps per `query` execution.
    max_query_vm_steps: u32,
    /// Wall-clock budget in whole seconds for one `query` execution.
    max_query_wall_clock_secs: u64,
    /// Largest single TEXT or BLOB column value `query` returns, in bytes.
    max_query_value_bytes: usize,
    /// Longest `structure` query text in UTF-8 bytes.
    max_structure_query_bytes: usize,
    /// Maximum capture matches returned on one `structure` page.
    max_structure_matches: usize,
    /// Wall-clock budget in whole seconds for one `structure` execution.
    max_structure_execution_time_secs: u64,
    /// Wall-clock budget in whole seconds for one sync operation.
    max_sync_wall_clock_secs: u64,
    /// Total bytes the result store keeps for paged results.
    max_result_store_bytes: u64,
    /// Seconds an unfinished paged result stays protected from reclaim after
    /// its last served page.
    result_active_grace_secs: u64,
}

/// Alias naming [`LimitOverrides`] when populated from `SLUGAUDIT_*` variables.
pub(crate) type EnvOverrides = LimitOverrides;

#[cfg(not(test))]
static ENV: OnceLock<EnvOverrides> = OnceLock::new();

/// Reads and validates `SLUGAUDIT_*` environment overrides (cached once per
/// process in production; read fresh per call under `#[cfg(test)]` so tests can
/// exercise startup/effective entrypoints across `temp_env` scopes).
pub(crate) fn env_overrides() -> Result<EnvOverrides, EnvConfigError> {
    #[cfg(not(test))]
    {
        if let Some(env) = ENV.get() {
            return Ok(*env);
        }
        let env = EnvOverrides::read_env()?;
        Ok(*ENV.get_or_init(|| env))
    }
    #[cfg(test)]
    {
        EnvOverrides::read_env()
    }
}

impl LimitOverrides {
    /// Reads every `SLUGAUDIT_*` limit variable, failing closed on invalid or
    /// `0` values.
    pub(crate) fn read_env() -> Result<Self, EnvConfigError> {
        Ok(Self {
            profile: profile_from_env()?,
            max_file_bytes: nonzero_override::<u64>("SLUGAUDIT_MAX_FILE_BYTES")?
                .map(|bytes| clamp_sqlite_value("SLUGAUDIT_MAX_FILE_BYTES", bytes))
                .transpose()?,
            max_parse_bytes: nonzero_override("SLUGAUDIT_MAX_PARSE_BYTES")?,
            max_parse_wall_clock_ms: nonzero_override("SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS")?,
            max_total_import_bytes: nonzero_override("SLUGAUDIT_MAX_TOTAL_IMPORT_BYTES")?,
            max_query_response_bytes: nonzero_override("SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES")?,
            default_page_bytes: nonzero_override("SLUGAUDIT_DEFAULT_PAGE_BYTES")?,
            max_query_sql_bytes: nonzero_override("SLUGAUDIT_MAX_QUERY_SQL_BYTES")?,
            max_query_vm_steps: nonzero_override("SLUGAUDIT_MAX_QUERY_VM_STEPS")?,
            max_query_wall_clock_secs: nonzero_override("SLUGAUDIT_MAX_QUERY_WALL_CLOCK_SECS")?,
            max_query_value_bytes: nonzero_override::<u64>("SLUGAUDIT_MAX_QUERY_VALUE_BYTES")?
                .map(env_query_value_bytes)
                .transpose()?,
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

    /// Applies this override layer onto `limits` without clamping
    /// `max_parse_bytes` yet (caller clamps once after all layers).
    pub(crate) fn apply_layer(
        &self,
        limits: &mut ResourceLimits,
        source: OverrideSource,
        result_store_pinned_earlier: bool,
    ) -> Result<(), LockPoisoned> {
        let (file_label, value_label) = match source {
            OverrideSource::GlobalFile => ("config max_file_bytes", "config max_query_value_bytes"),
            OverrideSource::Environment => (
                "SLUGAUDIT_MAX_FILE_BYTES",
                "SLUGAUDIT_MAX_QUERY_VALUE_BYTES",
            ),
        };
        if let Some(bytes) = self.max_file_bytes {
            limits.max_file_bytes = clamp_sqlite_value(file_label, bytes)?;
        }
        if let Some(bytes) = self.max_query_value_bytes {
            limits.max_query_value_bytes = clamp_sqlite_usize(value_label, bytes)?;
        }
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
        if !result_store_pinned_earlier {
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
        Ok(())
    }
}

fn set<T>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn env_query_value_bytes(bytes: u64) -> Result<usize, EnvConfigError> {
    const NAME: &str = "SLUGAUDIT_MAX_QUERY_VALUE_BYTES";
    usize::try_from(clamp_sqlite_value(NAME, bytes)?).map_err(|error| EnvConfigError::Invalid {
        variable: NAME.to_owned(),
        expected: format!("a value this platform can address ({error})"),
    })
}

fn nonzero_override<T>(name: &'static str) -> Result<Option<T>, EnvConfigError>
where
    T: std::str::FromStr + Default + PartialEq,
    T::Err: std::fmt::Display,
{
    match parse_override::<T>(name)? {
        Some(value) if value == T::default() => Err(EnvConfigError::Zero {
            variable: name.to_owned(),
        }),
        other => Ok(other),
    }
}

fn parse_override<T>(name: &str) -> Result<Option<T>, EnvConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let expected = || format!("a value of type {}", std::any::type_name::<T>());
    let raw = match std::env::var(name) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(EnvConfigError::NotUnicode {
                variable: name.to_owned(),
                expected: expected(),
            });
        }
    };
    raw.parse::<T>()
        .map(Some)
        .map_err(|error| EnvConfigError::Invalid {
            variable: name.to_owned(),
            expected: format!("{} ({error})", expected()),
        })
}

impl ResourceLimits {
    /// Clamps `max_parse_bytes` to `max_file_bytes` after all override layers,
    /// logging a warning once per configured value when clamped.
    pub(crate) fn clamp_parse_to_file_bytes(&mut self) -> Result<(), LockPoisoned> {
        if self.max_parse_bytes > self.max_file_bytes {
            if first_report("max_parse_bytes clamp", self.max_parse_bytes)? {
                tracing::warn!(
                    limit = "max_parse_bytes",
                    requested = self.max_parse_bytes,
                    cap = self.max_file_bytes,
                    "max_parse_bytes exceeds max_file_bytes; clamping",
                );
            }
            self.max_parse_bytes = self.max_file_bytes;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "limit_overrides_tests.rs"]
mod tests;
