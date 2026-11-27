//! The client model encoded by the registry: client specification record
//! ([`AgentDef`]), how slugaudit can register with a client ([`Strategy`]),
//! the on-disk shape a writer must produce ([`ConfigShape`]), where its config
//! lives ([`ConfigPath`]), how the entry's facts were established
//! ([`Evidence`]), and the argument grammar of its `mcp add` subcommand
//! ([`Dialect`]).
//!
//! [`crate::connect`] re-exports every type; the registry table and its
//! lookups live in `agents`.

use super::ConnectError;
use std::path::{Path, PathBuf};

/// The binary path as text a client can launch, or the typed refusal when it
/// is not valid UTF-8.
///
/// # Errors
///
/// A path that is not valid UTF-8 cannot be represented on a client's command
/// line or in its JSON config, so it is refused before any registration.
pub(crate) fn binary_text(binary: &Path) -> Result<&str, ConnectError> {
    binary.to_str().ok_or_else(|| {
        ConnectError::BinaryPath(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the binary path is not valid UTF-8, so it cannot be registered with a client",
        ))
    })
}

/// A specification for an AI agent or editor client supported by `slugaudit connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentDef {
    /// Stable registry identifier used on the command line (e.g. `claude`).
    pub id: &'static str,
    /// Human-readable client name shown in prompts and lists.
    pub display_name: &'static str,
    /// How slugaudit can register with this client.
    pub strategy: Strategy,
    /// The client's real executable: invocation and detection. Never a display
    /// name (warp's CLI is `oz`).
    pub cli: Option<&'static str>,
    /// An executable that can only *confirm* registration (`mcp list`), not
    /// create it — cursor's `agent`. Used for install detection only; never
    /// spawned, and never for add/remove.
    pub verify_cli: Option<&'static str>,
    /// Where the client's config file lives.
    pub config: ConfigPath,
    /// On-disk shape of the `slugaudit` entry the client reads.
    pub shape: ConfigShape,
    /// Argument grammar of the client's `mcp add` subcommand.
    pub dialect: Dialect,
    /// How this entry's facts were established.
    pub evidence: Evidence,
    /// Write the config file when the CLI is absent or its `mcp add` fails.
    pub file_fallback: bool,
    /// The client needs manual/UI steps beyond an automated write; `connect`
    /// refuses and prints [`AgentDef::guidance`] instead.
    pub manual_steps: bool,
    /// Paste-ready remedy shown when the automated path is refused, with
    /// `{binary}` standing for the slugaudit executable. Empty when the
    /// automated path is sufficient.
    pub remedy: &'static str,
    /// The version (and probe date) this entry's information comes from, e.g.
    /// `goose 1.52.0 (probed 2026-09-30)` or `vendor docs, as of 2026-09-30`.
    /// Never an invented version.
    pub reference: &'static str,
    /// The vendor page a human can re-check.
    pub docs_url: &'static str,
    /// Reconciliation note for docs: what is verified, contested, or pending.
    pub note: &'static str,
}

impl AgentDef {
    /// The sentence explaining why `connect` must refuse this entry, or `None`
    /// when the automated path is sufficient.
    #[must_use]
    pub fn blocker(&self) -> Option<&'static str> {
        if self.manual_steps {
            return Some(match self.shape {
                ConfigShape::GooseExtensions => {
                    "its config is YAML, which the JSON writer cannot merge without clobbering it"
                }
                ConfigShape::Unknown => "no machine-writable shape is established for its config",
                _ => "it requires a manual step slugaudit cannot perform",
            });
        }
        None
    }

    /// The manual remedy for a refusal, with `{binary}` resolved, or `None`
    /// when the entry carries no remedy. A path no client can represent yields
    /// the refusal text itself, never a lossy rendering.
    #[must_use]
    pub fn guidance(&self, binary: &Path) -> Option<String> {
        let remedy = self.remedy.trim();
        if remedy.is_empty() {
            return None;
        }
        Some(match binary_text(binary) {
            Ok(text) => remedy.replace("{binary}", text),
            Err(error) => error.to_string(),
        })
    }

    /// True when this entry registers itself by writing the config file, either
    /// as its primary strategy or as a fallback.
    #[must_use]
    pub fn writes_config_file(&self) -> bool {
        self.file_fallback
            || matches!(
                self.strategy,
                Strategy::ConfigFile | Strategy::CliInteractiveOnly
            )
    }

    /// Executables that prove the client is present: the invocation CLI, or a
    /// verify-only CLI.
    pub(super) fn executables(&self) -> impl Iterator<Item = &'static str> {
        self.cli.into_iter().chain(self.verify_cli)
    }
}

/// How slugaudit can register itself with a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// The client documents a scriptable add subcommand; slugaudit calls it.
    CliDirect,
    /// The client's only add path is an in-session/interactive command
    /// (e.g. OMP's `/mcp add`); an installer writes the config file instead.
    CliInteractiveOnly,
    /// No usable CLI; merge the config file.
    ConfigFile,
    /// Neither research pass established the mechanism. Never run an invented
    /// command: refuse and send the user to manual setup.
    Unknown,
}

/// The on-disk shape of the `slugaudit` entry a client reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigShape {
    /// top-level `"mcpServers": { name: { command, args } }`
    /// (cursor, windsurf, pi, omp, kimi, claude-desktop, bob, agy, warp)
    McpServersCommandArgs,
    /// top-level `"mcp": { name: { type: "local", command: [..], enabled: true } }`
    /// (opencode)
    OpenCodeLocal,
    /// top-level `"context_servers": { name: { command: "<string>", args: [..] } }`
    /// (zed — `command` is a STRING, sibling `args`)
    ZedContextServer,
    /// top-level `"extensions": { name: { type: "stdio", cmd, args } }`
    /// (goose, YAML).
    ///
    /// Deliberately has **no writer**. Two independent reasons, either of which
    /// is sufficient: (1) the ecosystem has no settled YAML crate, and the
    /// maintained option still carries a ported-C core — `serde_yaml` was
    /// archived in March 2024, and its continuation `yaml_serde` (published by
    /// the official YAML org; repo created Nov 2025, 8 releases to 0.10.7 in Aug
    /// 2026) vendors the C-derived libyaml parser/emitter in-tree with real
    /// `unsafe` (`src/libyaml/*.rs`),
    /// which sits badly with this crate's `#![forbid(unsafe_code)]` posture and
    /// its rule that transitive unsafe be named in the release notices; the
    /// pure-Rust alternatives (`saphyr`/`serde-saphyr`, `yaml-rust2`) are young
    /// with small adoption; and the solo fork `serde_yaml_ng`, which summaries
    /// often recommend, still depends on the deprecated `unsafe-libyaml` crate
    /// and has had no release since Sep 2025 — so none of these is a supply-chain
    /// decision this crate should take for one client's config; and (2) even a good
    /// parser would have to round-trip the user's file, dropping comments,
    /// ordering and anchors, i.e. it would break the very config it was trying
    /// to extend. So goose is instruction-only: the refusal prints a paste-ready
    /// block (see the `goose` entry's `remedy`). Do not "fix" this by adding a
    /// YAML dependency.
    GooseExtensions,
    /// The written shape could not be established; writers MUST refuse.
    Unknown,
}

impl ConfigShape {
    /// Top-level keys that may hold the server map for this shape. The legacy
    /// `mcp` alias is accepted alongside `mcpServers` so an entry written by an
    /// older slugaudit is still detected.
    #[must_use]
    pub fn sections(self) -> &'static [&'static str] {
        match self {
            ConfigShape::McpServersCommandArgs => &["mcpServers", "mcp"],
            ConfigShape::OpenCodeLocal => &["mcp"],
            ConfigShape::ZedContextServer => &["context_servers"],
            ConfigShape::GooseExtensions => &["extensions"],
            ConfigShape::Unknown => &[],
        }
    }

    /// The top-level key a writer must create, or `None` when this code cannot
    /// produce the file safely: `Unknown` has no established shape, and
    /// `GooseExtensions` is a YAML block the user pastes themselves — merging
    /// YAML without a parser would clobber an existing config.
    #[must_use]
    pub fn write_section(self) -> Option<&'static str> {
        match self {
            ConfigShape::McpServersCommandArgs => Some("mcpServers"),
            ConfigShape::OpenCodeLocal => Some("mcp"),
            ConfigShape::ZedContextServer => Some("context_servers"),
            ConfigShape::GooseExtensions | ConfigShape::Unknown => None,
        }
    }

    /// The `slugaudit` value a client reads for this shape.
    ///
    /// # Errors
    ///
    /// A binary path that is not valid UTF-8 is refused before any write: JSON
    /// cannot carry the raw bytes, and a lossy replacement-character path would
    /// be registered as a command the client cannot launch. A shape with no
    /// [`write_section`](Self::write_section) has no JSON entry and is refused
    /// with [`ConnectError::UnsupportedAgent`].
    pub fn entry(self, binary: &Path) -> Result<serde_json::Value, ConnectError> {
        let command = binary_text(binary)?;
        Ok(match self {
            ConfigShape::McpServersCommandArgs | ConfigShape::ZedContextServer => {
                serde_json::json!({ "command": command, "args": [] })
            }
            ConfigShape::OpenCodeLocal => {
                serde_json::json!({ "type": "local", "command": [command], "enabled": true })
            }
            ConfigShape::GooseExtensions | ConfigShape::Unknown => {
                return Err(ConnectError::UnsupportedAgent {
                    agent: format!("a client with a {self:?} config"),
                    reason: "its config has no JSON entry slugaudit can write".to_owned(),
                });
            }
        })
    }
}

/// How an entry's facts were established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Verified by probing an installed binary; overrides both doc passes.
    VerifiedByProbe,
    /// Verified from vendor/user documentation.
    DocVerified,
    /// Verified against the client's own source (stronger than docs, but not
    /// a runtime probe of an installed binary).
    VerifiedFromSource,
}

/// Where a client's config file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPath {
    /// Home-relative path, joined onto the OS home directory.
    Home(&'static str),
    /// Per-OS home-relative paths; `windows` may use the `%APPDATA%/` token.
    PerOs {
        /// Home-relative path on macOS.
        macos: &'static str,
        /// Home-relative path on Windows; `%APPDATA%/` expands to the
        /// roaming app-data directory.
        windows: &'static str,
        /// Home-relative path on Linux.
        linux: &'static str,
    },
    /// No verified path.
    Unverified,
}

impl ConfigPath {
    /// Resolves the config file against `home` (the OS home directory). `None`
    /// when no path is known: an unverified client, a platform without a
    /// documented path, or a Windows path whose `%APPDATA%` is unset.
    #[must_use]
    pub fn resolve(&self, home: &Path) -> Option<PathBuf> {
        match self {
            ConfigPath::Home(rel) => Some(home.join(rel)),
            ConfigPath::PerOs {
                macos,
                windows,
                linux,
            } => {
                // A platform outside the three documented ones has no path we
                // can act on: refuse and show the user the ones we know.
                if cfg!(target_os = "macos") {
                    expand_os_tokens(macos, home)
                } else if cfg!(target_os = "windows") {
                    expand_os_tokens(windows, home)
                } else if cfg!(target_os = "linux") {
                    expand_os_tokens(linux, home)
                } else {
                    None
                }
            }
            ConfigPath::Unverified => None,
        }
    }

    /// The per-OS paths for a platform-specific entry, so a refusal can print
    /// them instead of guessing.
    #[must_use]
    pub fn per_os_paths(&self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            ConfigPath::PerOs {
                macos,
                windows,
                linux,
            } => Some((macos, windows, linux)),
            _ => None,
        }
    }

    /// True when a path is known well enough to read or write.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !matches!(self, ConfigPath::Unverified)
    }
}

/// Expands the `%APPDATA%/` token Windows clients document; every other path
/// is already home-relative. `None` when the token is used but `APPDATA` is
/// unset or empty: the roaming directory is not guessed from `home`.
fn expand_os_tokens(rel: &str, home: &Path) -> Option<PathBuf> {
    match rel.strip_prefix("%APPDATA%/") {
        Some(rest) => std::env::var_os("APPDATA")
            .filter(|value| !value.is_empty())
            .map(|appdata| PathBuf::from(appdata).join(rest)),
        None => Some(home.join(rel)),
    }
}

/// Argument grammar of a client's `mcp add`/`mcp remove` subcommand. Only
/// meaningful for CLI-driven entries; config-only entries keep [`Dialect::Plain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `mcp add <name> <binary>`
    Plain,
    /// `mcp add -s user <name> -- <binary>` (Claude Code)
    Claude,
    /// `mcp add <name> --command <binary>` (Hermes)
    Hermes,
    /// `mcp add <name> -- <binary>` (Codex, Copilot, `OpenCode`, Amp, Pi)
    DashSeparator,
    /// `mcp add <name> --scope user -- <binary>` (Grok CLI)
    DashSeparatorUserScope,
    /// `mcp add --name <name> --command <binary>` (Amazon Q)
    Q,
    /// `mcp add <name> --command <binary>` (Crush)
    Crush,
    /// `mcp add <name> --transport stdio <binary>`. Required by OpenHands
    /// (`--transport {http,sse,stdio}` is mandatory there); accepted (stdio is
    /// already the default) by copilot, which also takes `--transport http|sse`.
    /// NOT accepted by opencode or amp, which infer the transport, and not by
    /// q, which has no transport flag at all.
    TransportStdio,
    /// `mcp add --type stdio <name> <binary>` (Droid). Droid spells the flag
    /// `--type` — there is no `--transport` alias — and the command is a plain
    /// positional, not a `--`-separated one.
    DroidType,
    /// `mcp add --transport stdio --scope global <name> <binary>` (IBM Bob).
    /// `-t/--transport` defaults to stdio and `-s/--scope` defaults to
    /// *workspace*, so a user-level registration must pass both explicitly.
    BobTransportScope,
    /// `mcp add -s user <name> <binary>` (Gemini CLI).
    ///
    /// Gemini's `--scope` defaults to **project**, which writes
    /// `./.gemini/settings.json` in the caller's working directory — so a
    /// global registration must pass `-s user` explicitly, or the entry lands
    /// somewhere the next session never reads. Unlike [`Dialect::Claude`] the
    /// command is a plain positional: Gemini has no `--` separator rule, and
    /// passing one would make `--` the command.
    GeminiScope,
}

#[cfg(test)]
#[path = "registry_types_tests.rs"]
mod tests;
