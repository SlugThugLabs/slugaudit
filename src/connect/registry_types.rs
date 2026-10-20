//! The client model encoded by the registry: how slugaudit can register with a
//! client ([`Strategy`]), the on-disk shape a writer must produce
//! ([`ConfigShape`]), where its config lives ([`ConfigPath`]), how the entry's
//! facts were established ([`Evidence`]), and the argument grammar of its
//! `mcp add` subcommand ([`Dialect`]).
//!
//! [`crate::connect_agents`] re-exports every type at its historical path; the
//! registry table and its lookups stay there.

use crate::cli::ConnectError;
use std::path::{Path, PathBuf};

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
    /// be registered as a command the client cannot launch.
    pub fn entry(self, binary: &Path) -> Result<serde_json::Value, ConnectError> {
        let command = binary.to_str().ok_or_else(|| {
            ConnectError::BinaryPath(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the binary path is not valid UTF-8, so it cannot be written to a client config",
            ))
        })?;
        Ok(match self {
            ConfigShape::McpServersCommandArgs => {
                serde_json::json!({ "command": command, "args": [] })
            }
            ConfigShape::OpenCodeLocal => {
                serde_json::json!({ "type": "local", "command": [command], "enabled": true })
            }
            ConfigShape::ZedContextServer => {
                serde_json::json!({ "command": command, "args": [] })
            }
            ConfigShape::GooseExtensions => {
                serde_json::json!({ "type": "stdio", "cmd": command, "args": [] })
            }
            ConfigShape::Unknown => serde_json::Value::Null,
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
    /// Resolves the config file against `home` (the OS home directory).
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
                    Some(expand_os_tokens(macos, home))
                } else if cfg!(target_os = "windows") {
                    Some(expand_os_tokens(windows, home))
                } else if cfg!(target_os = "linux") {
                    Some(expand_os_tokens(linux, home))
                } else {
                    None
                }
            }
            ConfigPath::Unverified => None,
        }
    }

    /// The per-OS paths for a platform-specific entry, so a refusal can print
    /// them instead of guessing.
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
    pub fn is_usable(&self) -> bool {
        !matches!(self, ConfigPath::Unverified)
    }
}

/// Expands the `%APPDATA%/` token Windows clients document; every other path
/// is already home-relative.
fn expand_os_tokens(rel: &str, home: &Path) -> PathBuf {
    match rel.strip_prefix("%APPDATA%/") {
        Some(rest) => match std::env::var_os("APPDATA") {
            Some(appdata) => PathBuf::from(appdata).join(rest),
            None => home.join("AppData/Roaming").join(rest),
        },
        None => home.join(rel),
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
    /// `mcp add <name> [--scope ...] -- <binary>`
    DashSeparator,
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
