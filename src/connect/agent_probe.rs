//! Host filesystem and `PATH` probing for client installation and connection
//! status, plus `#`-comment-aware scanning for YAML, TOML, and shell-rc files.

use super::agents::AGENTS;
use super::registry_types::{AgentDef, ConfigPath, ConfigShape, Strategy};
use std::path::Path;

impl AgentDef {
    /// Returns true if this agent appears to be installed on the machine.
    ///
    /// # Errors
    ///
    /// A config path that cannot be examined for a reason other than "not
    /// found" (permissions, a path component that is a file) is returned as a
    /// [`ProbeFailure`] with its source, never read as "absent".
    pub fn is_installed(&self, home: Option<&Path>) -> Result<bool, ProbeFailure> {
        if self.executables().any(crate::probe::cli_installed) {
            return Ok(true);
        }
        let Some(home) = home else { return Ok(false) };
        let Some(path) = self.config.resolve(home) else {
            return Ok(false);
        };
        if self.config_path_exists(&path)? {
            return Ok(true);
        }
        match path.parent() {
            Some(parent) if parent != home => self.config_path_exists(parent),
            _ => Ok(false),
        }
    }

    fn config_path_exists(&self, path: &Path) -> Result<bool, ProbeFailure> {
        path.try_exists()
            .map_err(|source| ProbeFailure::for_agent(self.id, "installed", path, source))
    }

    /// Returns true if this agent's local config file already has SlugAudit.
    ///
    /// Config files only: no client CLI is ever spawned to ask. A client with
    /// no known config file (`ConfigPath::Unverified`: agy, amp, q) is always
    /// reported as not detected.
    ///
    /// # Errors
    ///
    /// A config that cannot be examined (unreadable, oversized, not a regular
    /// file, or malformed JSON for a JSON client) is returned as a
    /// [`ProbeFailure`] with its source, never read as "not connected".
    pub fn is_connected(&self, home: Option<&Path>) -> Result<bool, ProbeFailure> {
        let Some(home) = home else { return Ok(false) };
        self.config_has_slugaudit(home)
    }

    pub(super) fn config_has_slugaudit(&self, home: &Path) -> Result<bool, ProbeFailure> {
        let Some(path) = self.config.resolve(home) else {
            return Ok(false);
        };
        let is_toml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
        let result = if matches!(
            self.shape,
            ConfigShape::GooseExtensions | ConfigShape::Unknown
        ) {
            super::config_file::readable_config(&path).map(|text| {
                text.is_some_and(|content| line_oriented_has_slugaudit(&content, is_toml))
            })
        } else {
            super::config_file::has_slugaudit(&path, self.shape.sections())
        };
        result.map_err(|source| ProbeFailure::for_agent(self.id, "connected", &path, source))
    }
}

/// Strips `#` comments from one TOML, YAML, or shell-rc line while preserving
/// `#` characters inside single- or double-quoted strings. In TOML any unquoted
/// `#` begins a comment; in YAML and POSIX shell `#` begins a comment at line
/// start or after whitespace.
fn strip_hash_comment(line: &str, any_unquoted_hash: bool) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut after_ws_or_bol = true;
    for (idx, ch) in line.char_indices() {
        if in_single {
            if ch == '\'' {
                in_single = false;
                after_ws_or_bol = false;
            }
            continue;
        }
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
                after_ws_or_bol = false;
            }
            continue;
        }
        match ch {
            '\'' => {
                in_single = true;
                after_ws_or_bol = false;
            }
            '"' => {
                in_double = true;
                after_ws_or_bol = false;
            }
            '#' if any_unquoted_hash || after_ws_or_bol => {
                return match line.get(..idx) {
                    Some(prefix) => prefix,
                    None => line,
                };
            }
            other => {
                after_ws_or_bol = other.is_whitespace();
            }
        }
    }
    line
}

/// True when any non-comment portion of a YAML, TOML, or rc config mentions `slugaudit`.
fn line_oriented_has_slugaudit(text: &str, is_toml: bool) -> bool {
    text.lines()
        .map(|line| strip_hash_comment(line, is_toml))
        .any(|code| code.contains("slugaudit"))
}

/// A client whose install or connection state could not be determined: the
/// config path that could not be examined, and why.
#[derive(Debug)]
pub struct ProbeFailure {
    /// The registry identifier of the client being probed.
    pub agent: &'static str,
    /// What was being checked: `"installed"` or `"connected"`.
    pub check: &'static str,
    /// The path that could not be examined.
    pub path: std::path::PathBuf,
    /// The underlying I/O failure.
    pub source: std::io::Error,
}

impl ProbeFailure {
    fn for_agent(
        agent: &'static str,
        check: &'static str,
        path: &Path,
        source: std::io::Error,
    ) -> Self {
        Self {
            agent,
            check,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl std::fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "could not tell whether {} is {}: {}: {}",
            self.agent,
            self.check,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ProbeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The result of scanning for installed clients.
#[derive(Debug)]
pub struct Detection {
    /// Agents that appear to be installed on the machine.
    pub agents: Vec<AgentDef>,
    /// Agents that could not be checked; they are not in `agents`.
    pub failures: Vec<ProbeFailure>,
}

/// Returns all agents in [`AGENTS`] that appear to be installed on the machine.
/// An agent whose config path cannot be examined is left out of `agents` and
/// returned in `failures`; the caller reports each failure once.
#[must_use]
pub fn detected_agents(home: Option<&Path>) -> Detection {
    let mut agents = Vec::new();
    let mut failures = Vec::new();
    for agent in AGENTS {
        match agent.is_installed(home) {
            Ok(true) => agents.push(*agent),
            Ok(false) => {}
            Err(failure) => failures.push(failure),
        }
    }
    Detection { agents, failures }
}

/// How an interactive disconnect candidate was surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisconnectCandidate {
    /// Local config file confirmed an active `slugaudit` registration.
    Connected(AgentDef),
    /// CLI-managed client (`ConfigPath::Unverified`) whose executable is installed.
    CliManaged(AgentDef),
}

/// Categorized scan of all registered agents for interactive `disconnect`.
pub(super) struct DisconnectScan {
    pub(super) candidates: Vec<DisconnectCandidate>,
    pub(super) manual_only: Vec<AgentDef>,
    pub(super) unchecked: Vec<ProbeFailure>,
}

/// Scans [`AGENTS`] for active connections, manual-only entries, and CLI-managed
/// clients without local config paths.
pub(super) fn scan_disconnect_candidates(home: Option<&Path>) -> DisconnectScan {
    let mut candidates = Vec::new();
    let mut manual_only = Vec::new();
    let mut unchecked = Vec::new();
    for agent in AGENTS {
        match agent.is_connected(home) {
            Ok(true) if agent.manual_steps => manual_only.push(*agent),
            Ok(true) => candidates.push(DisconnectCandidate::Connected(*agent)),
            Ok(false)
                if matches!(agent.strategy, Strategy::CliDirect)
                    && matches!(agent.config, ConfigPath::Unverified) =>
            {
                match agent.is_installed(home) {
                    Ok(true) => candidates.push(DisconnectCandidate::CliManaged(*agent)),
                    Ok(false) => {}
                    Err(failure) => unchecked.push(failure),
                }
            }
            Ok(false) => {}
            Err(failure) => unchecked.push(failure),
        }
    }
    DisconnectScan {
        candidates,
        manual_only,
        unchecked,
    }
}
