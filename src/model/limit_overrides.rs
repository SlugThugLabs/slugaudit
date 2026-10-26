//! Hygiene for limit overrides (environment and project config): a zero that
//! would silently disable work is refused, and a warning about one cause is
//! logged once per process rather than once per load.
//!
//! The limits are rebuilt from the project config on every tool call and
//! every sync, so a warning issued at each build repeats for one unchanged
//! setting (the clamp warning was logged three times for one cause). The
//! ledger below remembers which (setting, configured value) pairs were
//! already reported; a setting whose value changes is a new cause and warns
//! again.

use super::ResourceLimits;
use crate::util::lock_or_recover;
use std::collections::BTreeSet;
use std::sync::Mutex;

/// The (setting, configured value) pairs already warned about.
static REPORTED: Mutex<BTreeSet<(&'static str, u64)>> = Mutex::new(BTreeSet::new());

/// Serialises the tests that count ledger-backed warnings. Each takes it through
/// [`fresh_ledger`], so none sees a warning another test already spent.
#[cfg(test)]
static LEDGER_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Takes [`LEDGER_TEST_LOCK`] and empties the ledger, so the holder starts from
/// a known state and the others wait for it. Hold the guard for the whole test.
#[cfg(test)]
pub(super) fn fresh_ledger() -> std::sync::MutexGuard<'static, ()> {
    let guard = lock_or_recover(&LEDGER_TEST_LOCK, "limit_override_test_lock");
    lock_or_recover(&REPORTED, "limit_override_warnings").clear();
    guard
}

/// `true` the first time this process sees `(setting, value)`.
fn first_report(setting: &'static str, value: u64) -> bool {
    lock_or_recover(&REPORTED, "limit_override_warnings").insert((setting, value))
}

/// Passes `value` through unless it is zero. Zero is meaningless for the
/// limits this guards (it would make every file oversize, every parse time
/// out, every query too long), so it is refused with a warning naming
/// `setting` (an environment variable or a `config.json` key), once per
/// process, and the caller keeps the value already in place.
pub(super) fn refuse_zero<T: Default + PartialEq>(setting: &'static str, value: T) -> Option<T> {
    if value != T::default() {
        return Some(value);
    }
    if first_report(setting, 0) {
        tracing::warn!(
            setting,
            "limit override is 0, which is not a usable limit; it was ignored and the current value applies",
        );
    }
    None
}

impl ResourceLimits {
    /// [`refuse_zero`] for a `config.json` key; `None` stays `None`.
    pub(crate) fn config_nonzero<T: Default + PartialEq>(
        key: &'static str,
        value: Option<T>,
    ) -> Option<T> {
        refuse_zero(key, value?)
    }

    /// Clamps `max_parse_bytes` to `max_file_bytes` after every override.
    ///
    /// A file over `max_file_bytes` is never read, so a parse cap above it can
    /// never act; keeping it would report a limit that is not the one in
    /// force. Runs after the env overrides and again after the project config
    /// (`ProjectConfig::apply_to_limits`), and says so when it acts (once per
    /// configured value), like the SQLite clamp.
    pub(crate) fn clamp_parse_to_file_bytes(&mut self) {
        if self.max_parse_bytes > self.max_file_bytes {
            if first_report("max_parse_bytes clamp", self.max_parse_bytes) {
                tracing::warn!(
                    limit = "max_parse_bytes",
                    requested = self.max_parse_bytes,
                    cap = self.max_file_bytes,
                    "max_parse_bytes exceeds max_file_bytes; clamping",
                );
            }
            self.max_parse_bytes = self.max_file_bytes;
        }
    }
}

#[cfg(test)]
#[path = "limit_overrides_tests.rs"]
mod tests;
