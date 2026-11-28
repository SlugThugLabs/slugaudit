//! Error taxonomy for `connect` and `disconnect` operations.

use std::path::PathBuf;
use thiserror::Error;

/// Errors encountered while connecting or disconnecting an agent client.
#[derive(Debug, Error)]
pub enum ConnectError {
    /// The user picked an option outside the presented list.
    #[error("invalid agent choice; choose one of the listed options")]
    InvalidChoice,
    /// Standard input closed before the prompt was answered.
    #[error("input ended before an answer was given")]
    EndOfInput,
    /// The running binary's path could not be resolved.
    #[error("binary path unavailable: {0}")]
    BinaryPath(std::io::Error),
    /// Neither a home directory nor a usable config parent was available.
    #[error("home directory is unavailable")]
    HomeUnavailable,
    /// No registry entry matches the requested agent name.
    #[error("unknown agent {0:?}; run `slugaudit connect` to see detected agents")]
    UnknownAgent(String),
    /// The client's executable could not be spawned or waited on.
    #[error("failed to run `{cli}`: {inner}")]
    AgentCommand {
        /// Client executable that could not be run.
        cli: String,
        /// Underlying spawn or wait error.
        inner: std::io::Error,
    },
    /// The client's executable ran past its deadline and was killed.
    #[error("`{cli}` did not finish within {timeout_secs}s and was stopped")]
    AgentTimeout {
        /// Client executable that timed out.
        cli: String,
        /// The deadline it exceeded, in seconds.
        timeout_secs: f32,
    },
    /// The client's `mcp remove` subcommand exited non-zero.
    #[error("`{cli} mcp remove` exited with {status}")]
    RemoveFailed {
        /// Client executable whose remove failed.
        cli: String,
        /// Exit status as printed by the OS.
        status: String,
    },
    /// The client's `mcp add` subcommand exited non-zero after printing output.
    #[error("`{cli} mcp add` exited with {status} — see the output above")]
    AddFailed {
        /// Client executable whose add failed.
        cli: String,
        /// Exit status as printed by the OS.
        status: String,
    },
    /// The client rejected `mcp add` and printed nothing, so there is no
    /// "output above" to point the user at.
    #[error("`{cli} mcp add` exited with {status} and printed no output")]
    AddFailedNoOutput {
        /// Client executable whose add failed.
        cli: String,
        /// Exit status as printed by the OS.
        status: String,
    },
    /// The client has no automated registration path; carries the reason the
    /// caller reports.
    #[error("{agent} cannot be connected automatically: {reason}")]
    UnsupportedAgent {
        /// Human-readable client name.
        agent: String,
        /// Why the automated path is refused.
        reason: String,
    },
    /// The client's config file still registers `slugaudit`, but its CLI
    /// executable is not on `PATH` and SlugAudit does not rewrite this config.
    #[error(
        "cannot disconnect from {agent}: {path} still has `slugaudit`, \
         but `{cli}` is not on PATH; install `{cli}` and retry, or remove the \
         `slugaudit` entry from {path} manually"
    )]
    DisconnectCliMissing {
        /// Human-readable client name.
        agent: String,
        /// Client CLI executable expected on `PATH`.
        cli: String,
        /// Config path (or client target) that still contains `slugaudit`.
        path: String,
    },
    /// Probing a client's local configuration failed.
    #[error("{0}")]
    Probe(#[from] super::ProbeFailure),
    /// The config's expected section exists but is not a JSON object.
    #[error(
        "cannot connect: \"{section}\" in the configuration is not an object, so there is \
         nowhere to write the slugaudit entry. Fix or remove that key, then retry"
    )]
    SectionNotAnObject {
        /// Top-level section key that is not an object.
        section: String,
    },
    /// The config file's top-level JSON value is not an object.
    #[error(
        "cannot connect: {path} does not hold a JSON object at its top level, so there is \
         nowhere to write the slugaudit entry. Fix or remove the file, then retry"
    )]
    RootNotAnObject {
        /// Config file that could not be written.
        path: PathBuf,
    },
    /// An existing `.jsonc` config file contains comments that an automated
    /// JSON rewrite would destroy.
    #[error(
        "cannot modify {path} automatically because it contains comments that rewriting \
         would erase; {advice}"
    )]
    CommentedConfig {
        /// Config file whose comments were preserved.
        path: PathBuf,
        /// Manual instruction naming where to paste or remove the entry.
        advice: String,
    },
    /// An existing config file could not be parsed as JSON.
    #[error(
        "cannot modify {path}: configuration file exists but contains invalid JSON: {source} (fix syntax/comments or remove the file to reset, then retry)"
    )]
    InvalidConfig {
        /// Config file that failed to parse.
        path: PathBuf,
        /// Parser error, including line and column.
        #[source]
        source: serde_json::Error,
    },
    /// A JSON value could not be encoded or decoded.
    #[error("JSON config error: {0}")]
    Json(#[from] serde_json::Error),
    // Spelled out rather than `transparent`: transparent forwards `source()`
    // through the io::Error, which hides the error itself from
    // `is_broken_pipe`'s chain walk.
    /// An I/O operation failed. A closed pipe is treated as a clean exit.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}
