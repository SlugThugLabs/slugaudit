//! Hygiene for limit overrides: a zero in an environment variable is a
//! startup error; a zero in the global config is ignored with a warning.
//! A warning about one cause is logged once per process rather than once per
//! load.
//!
//! The limits are rebuilt from the global config on every tool call and
//! every sync, so a warning issued at each build would repeat for one
//! unchanged setting. The
//! ledger below remembers which (setting, configured value) pairs were
//! already reported; a setting whose value changes is a new cause and warns
//! again.

use super::limits::SQLITE_MAX_VALUE_BYTES;
use super::{EnvConfigError, ResourceLimits};
use crate::util::{LockPoisoned, lock};
use std::collections::BTreeSet;
use std::sync::Mutex;

/// The ledger's contents: (setting, configured value) pairs.
type Ledger = BTreeSet<(&'static str, u64)>;

/// The (setting, configured value) pairs already warned about.
#[cfg(not(test))]
static REPORTED: Mutex<Ledger> = Mutex::new(BTreeSet::new());

// In tests the ledger is per thread. A process-wide ledger would let any
// unrelated test that builds limits while another test's environment held a
// zero spend that test's one warning outside its capture, and the zero tests
// would then see no warning at all.
#[cfg(test)]
thread_local! {
    static REPORTED: Mutex<Ledger> = const { Mutex::new(BTreeSet::new()) };
}

/// Runs `change` on the process-wide ledger.
#[cfg(not(test))]
fn with_ledger<T>(change: impl FnOnce(&mut Ledger) -> T) -> Result<T, LockPoisoned> {
    Ok(change(&mut *lock(&REPORTED, "limit_override_warnings")?))
}

/// Runs `change` on this thread's ledger.
#[cfg(test)]
fn with_ledger<T>(change: impl FnOnce(&mut Ledger) -> T) -> Result<T, LockPoisoned> {
    REPORTED.with(|ledger| Ok(change(&mut *lock(ledger, "limit_override_warnings")?)))
}

/// Empties this thread's ledger so the calling test starts from a known state.
#[cfg(test)]
pub(super) fn fresh_ledger() -> Result<(), LockPoisoned> {
    with_ledger(BTreeSet::clear)
}

/// `true` the first time the ledger sees `(setting, value)`. A poisoned
/// ledger is an error: whether the warning is due is unknown.
fn first_report(setting: &'static str, value: u64) -> Result<bool, LockPoisoned> {
    with_ledger(|reported| reported.insert((setting, value)))
}

/// Clamps one single-value limit to what the bundled SQLite can store.
///
/// A value above `SQLITE_MAX_VALUE_BYTES` is unusable: it would be rejected
/// at insert or read. It is clamped, with a warning naming `setting` (an
/// environment variable or a `config.json` key) logged once per configured
/// value, rather than letting a per-file skip turn into a failed publish.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the warning ledger was poisoned.
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

/// Passes `value` through unless it is zero. Zero is meaningless for the
/// limits this guards (it would make every file oversize, every parse time
/// out, every query too long), so it is refused with a warning naming
/// `setting` (a `config.json` key), once per process, and the caller keeps the
/// value already in place.
///
/// # Errors
///
/// Returns [`LockPoisoned`] when the warning ledger was poisoned.
pub(super) fn refuse_zero<T: Default + PartialEq>(
    setting: &'static str,
    value: T,
) -> Result<Option<T>, LockPoisoned> {
    if value != T::default() {
        return Ok(Some(value));
    }
    if first_report(setting, 0)? {
        tracing::warn!(
            setting,
            "limit override is 0, which is not a usable limit; it was ignored and the current value applies",
        );
    }
    Ok(None)
}

/// [`parse_override`] for a limit where zero is meaningless: a `0` is an
/// error naming the variable (never a value), so startup fails closed rather
/// than running with a limit the operator did not ask for.
pub(super) fn nonzero_override<T>(name: &'static str) -> Result<Option<T>, EnvConfigError>
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

/// Parses one `SLUGAUDIT_<FIELD>` override.
///
/// An unset variable is not a failure: `Ok(None)`, the caller keeps the
/// default ("only explicitly-set variables override"). A variable that IS
/// set but cannot be read or parsed is operator intent that cannot be
/// honored, so it is an error naming the variable and what is valid, never
/// the value (`NotUnicode`'s Display prints the raw value, which may be
/// sensitive).
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
    /// [`refuse_zero`] for a `config.json` key; `None` stays `None`.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the warning ledger was poisoned.
    pub(crate) fn config_nonzero<T: Default + PartialEq>(
        key: &'static str,
        value: Option<T>,
    ) -> Result<Option<T>, LockPoisoned> {
        match value {
            Some(value) => refuse_zero(key, value),
            None => Ok(None),
        }
    }

    /// Clamps `max_parse_bytes` to `max_file_bytes` after every override.
    ///
    /// A file over `max_file_bytes` is never read, so a parse cap above it can
    /// never act; keeping it would report a limit that is not the one in
    /// force. Runs after the env overrides (`apply_env_overrides`), and says so
    /// when it acts (once per configured value), like the SQLite clamp.
    ///
    /// # Errors
    ///
    /// Returns [`LockPoisoned`] when the warning ledger was poisoned.
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
