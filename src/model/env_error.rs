//! The startup error for a set-but-invalid environment variable.

use super::system_ram::MemoryDetectError;

/// A `SLUGAUDIT_*` or `RUST_LOG` variable the operator set that cannot be
/// honored. Startup exits non-zero on it instead of running with other
/// limits than the operator asked for. No variant carries the variable's
/// value, which may be sensitive: only its name and what is valid.
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
    /// The variable is set but its value is not valid.
    #[error("{variable} is set but is not valid; expected {expected}")]
    Invalid {
        /// Name of the variable.
        variable: String,
        /// Description of what is valid.
        expected: String,
    },
    /// The Adaptive profile sizes limits from host memory, which could not
    /// be detected, so a profile that needs no detection must be chosen.
    #[error(
        "the adaptive profile needs the host memory size, which could not be detected ({source}); set SLUGAUDIT_PROFILE to `lean` or `audit`"
    )]
    MemoryUndetectable {
        /// Why detection failed.
        #[source]
        source: MemoryDetectError,
    },
}
