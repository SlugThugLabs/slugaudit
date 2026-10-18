//! Supported AI agents and editors registry for SlugAudit connection.
//!
//! Every entry records *how* slugaudit can register itself with a client
//! ([`Strategy`]), the client's real executable, where its config lives
//! ([`ConfigPath`]), and the on-disk shape a writer must produce
//! ([`ConfigShape`]). [`Dialect`] is left with one job only: the argument
//! grammar of a client's `mcp add` subcommand.
//!
//! Each entry carries an [`Evidence`] marker because two independent
//! doc-research passes disagreed on seven of the original 26 clients — and each
//! pass was wrong on roughly half of those (cursor and kimi turned out to have
//! no `mcp add` at all, while opencode and crush landed on the opposite verdict
//! each pass had chosen). Only probing the installed binaries was reliable, so
//! a client whose mechanism could not be established is not in the registry
//! (pearai, trae and 1mcp were removed on 2026-10-17 for that reason), and
//! `connect` never runs an invented command for a [`Strategy::Unknown`] one. A
//! probe result ([`Evidence::VerifiedByProbe`]) overrides both doc passes.

use std::path::{Path, PathBuf};

// slugaudit-line-exception: approved-by=slugthug; reason=the client registry is a data table with one line per client (23 rows) plus the three-state strategy, written-shape and evidence model it encodes; splitting the table from its invariants would scatter the reconciliation this file exists to record and that connect_exec, docs and tests all read from one place

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
    pub fn entry(self, binary: &Path) -> serde_json::Value {
        let command = binary.to_string_lossy();
        match self {
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
        }
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
        macos: &'static str,
        windows: &'static str,
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

/// A specification for an AI agent or editor client supported by `slugaudit connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentDef {
    pub id: &'static str,
    pub display_name: &'static str,
    pub strategy: Strategy,
    /// The client's real executable: invocation and detection. Never a display
    /// name (warp's CLI is `oz`).
    pub cli: Option<&'static str>,
    /// An executable that can only *confirm* registration (`mcp list`), not
    /// create it — cursor's `agent`. Used for detection and `is_connected`
    /// only; never for add/remove.
    pub verify_cli: Option<&'static str>,
    pub config: ConfigPath,
    pub shape: ConfigShape,
    pub dialect: Dialect,
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
    /// when the entry carries no remedy.
    pub fn guidance(&self, binary: &Path) -> Option<String> {
        let remedy = self.remedy.trim();
        (!remedy.is_empty()).then(|| remedy.replace("{binary}", &binary.display().to_string()))
    }
    /// True when this entry registers itself by writing the config file, either
    /// as its primary strategy or as a fallback.
    pub fn writes_config_file(&self) -> bool {
        self.file_fallback
            || matches!(
                self.strategy,
                Strategy::ConfigFile | Strategy::CliInteractiveOnly
            )
    }

    /// Executables that prove the client is present: the invocation CLI, or a
    /// verify-only CLI.
    fn executables(&self) -> impl Iterator<Item = &'static str> {
        self.cli.into_iter().chain(self.verify_cli)
    }

    /// Returns true if this agent appears to be installed on the machine.
    pub fn is_installed(&self, home: Option<&Path>) -> bool {
        if self.executables().any(|cli| which::which(cli).is_ok()) {
            return true;
        }
        let Some(home) = home else { return false };
        let Some(path) = self.config.resolve(home) else {
            return false;
        };
        if path.exists() {
            return true;
        }
        path.parent().is_some_and(|p| p.exists() && p != home)
    }

    /// Returns true if this agent already has SlugAudit configured.
    pub fn is_connected(&self, home: Option<&Path>) -> bool {
        if let Some(home) = home
            && self.config_has_slugaudit(home)
        {
            return true;
        }
        if let Some(cli) = self.executables().find(|cli| which::which(cli).is_ok()) {
            return crate::connect_cli::check_connected(cli);
        }
        false
    }

    fn config_has_slugaudit(&self, home: &Path) -> bool {
        let Some(path) = self.config.resolve(home) else {
            return false;
        };
        if self.shape == ConfigShape::GooseExtensions {
            // Goose reads YAML; only a textual scan is safe for the written
            // JSON-as-YAML (and it also catches a hand-written block entry).
            return crate::util::read_file_capped(&path, crate::util::MAX_FOREIGN_TEXT_BYTES)
                .is_ok_and(|c| c.contains("slugaudit"));
        }
        crate::connect_exec::has_slugaudit(&path, self.shape.sections())
    }
}

/// Static catalog of known AI coding agents and MCP clients.
#[rustfmt::skip]
pub const AGENTS: &[AgentDef] = &[
    AgentDef { id: "agy", display_name: "Antigravity (agy)", strategy: Strategy::CliDirect, cli: Some("agy"), verify_cli: None, config: ConfigPath::Unverified, shape: ConfigShape::Unknown, dialect: Dialect::Plain, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "agy 1.2.14 (probed 2026-09-30)", docs_url: "https://antigravity.google/docs/mcp/", note: "`agy mcp add|remove|list|enable|disable` all exist (probed on the installed binary).", },
    AgentDef { id: "gemini", display_name: "Gemini CLI", strategy: Strategy::CliDirect, cli: Some("gemini"), verify_cli: None, config: ConfigPath::Home(".gemini/settings.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::GeminiScope, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "gemini 0.64.0 (probed 2026-10-17)", docs_url: "https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md", note: "probed against the real CLI: `gemini mcp add <name> <commandOrUrl> [args...]`; `--scope` defaults to project (writes `./.gemini/settings.json` in the CWD), so registration passes `-s user` to land in `~/.gemini/settings.json`; no `--` separator is used because Gemini takes the command as a positional. End to end, `connect gemini` wrote the entry and `disconnect gemini` removed it. Gemini lists the server as Disabled until the folder is trusted. The settings file is detection only.", },
    AgentDef { id: "claude", display_name: "Claude Code", strategy: Strategy::CliDirect, cli: Some("claude"), verify_cli: None, config: ConfigPath::Home(".claude.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Claude, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "claude 2.1.233 (probed 2026-09-30)", docs_url: "https://code.claude.com/docs/en/mcp", note: "`claude mcp add` probed; user scope is documented as `-s user`; `.claude.json` is detection only.", },
    AgentDef { id: "hermes", display_name: "Hermes Agent", strategy: Strategy::CliDirect, cli: Some("hermes"), verify_cli: None, config: ConfigPath::Home(".hermes/config.yaml"), shape: ConfigShape::Unknown, dialect: Dialect::Hermes, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "hermes 0.18.2 (probed 2026-09-30)", docs_url: "https://hermes-agent.nousresearch.com/docs/reference/mcp-config-reference", note: "`hermes mcp add <name> --command <binary>` probed; the YAML config is detection only.", },
    AgentDef { id: "copilot", display_name: "GitHub Copilot", strategy: Strategy::CliDirect, cli: Some("copilot"), verify_cli: None, config: ConfigPath::Home(".copilot/mcp-config.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::DashSeparator, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "copilot 1.0.89 (probed)", docs_url: "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers", note: "probed: `copilot mcp add <name> -- <cmd>` matches slugaudit's emission and adds no `--scope`; the config file is detection only.", },
    AgentDef { id: "codex", display_name: "Codex CLI", strategy: Strategy::CliDirect, cli: Some("codex"), verify_cli: None, config: ConfigPath::Home(".codex/config.toml"), shape: ConfigShape::Unknown, dialect: Dialect::DashSeparator, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "codex 0.149.1 (probed 2026-09-30)", docs_url: "https://developers.openai.com/codex/mcp/", note: "`codex mcp add` probed; the TOML config is detection only.", },
    AgentDef { id: "opencode", display_name: "OpenCode", strategy: Strategy::CliDirect, cli: Some("opencode"), verify_cli: None, config: ConfigPath::Home(".config/opencode/opencode.jsonc"), shape: ConfigShape::OpenCodeLocal, dialect: Dialect::DashSeparator, evidence: Evidence::VerifiedByProbe, file_fallback: true, manual_steps: false, remedy: "", reference: "opencode 1.18.33 (probed)", docs_url: "https://opencode.ai/docs/mcp-servers/", note: "probed: `opencode mcp add <name> -- <command> [args]` works (the `--` form is absent from --help but named in its own error text); no --type/--transport (transport is inferred); it wrote `.jsonc` (kept as the file fallback), where its own entry omits `enabled`.", },
    AgentDef { id: "amp", display_name: "Amp Agent", strategy: Strategy::CliDirect, cli: Some("amp"), verify_cli: None, config: ConfigPath::Unverified, shape: ConfigShape::Unknown, dialect: Dialect::DashSeparator, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "amp 0.0.1790778300-g865493 (probed)", docs_url: "https://ampcode.com/docs/customize/mcp", note: "probed: `amp mcp add <name> -- <command>` works (`--settings-file` also exists).", },
    AgentDef { id: "bob", display_name: "Bob Agent", strategy: Strategy::CliDirect, cli: Some("bob"), verify_cli: None, config: ConfigPath::Home(".bob/mcp_settings.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::BobTransportScope, evidence: Evidence::VerifiedByProbe, file_fallback: true, manual_steps: false, remedy: "", reference: "bob 2.0.5 (probed 2026-09-30)", docs_url: "https://bob.ibm.com/docs/shell/configuration/mcp/mcp-bobshell", note: "probed: both doc passes missed it, but `bob mcp add <name> <commandOrUrl>` exists with -t/--transport stdio|http (default stdio) and -s/--scope workspace|global (default workspace); the global file is the fallback path.", },
    AgentDef { id: "grok", display_name: "Grok CLI", strategy: Strategy::CliDirect, cli: Some("grok"), verify_cli: None, config: ConfigPath::Home(".grok/config.toml"), shape: ConfigShape::Unknown, dialect: Dialect::DashSeparator, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "grok 1.0.44 (probed)", docs_url: "https://docs.x.ai/build/features/mcp-servers", note: "probed: `grok mcp add` and `--scope user` are real (user scope is the explicit default); the TOML config is detection only.", },
    AgentDef { id: "goose", display_name: "Goose CLI", strategy: Strategy::ConfigFile, cli: Some("goose"), verify_cli: None, config: ConfigPath::Home(".config/goose/config.yaml"), shape: ConfigShape::GooseExtensions, dialect: Dialect::Plain, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: true, remedy: "goose has no `mcp add`. Add this block to `~/.config/goose/config.yaml`:\nextensions:\n  slugaudit:\n    type: stdio\n    cmd: {binary}\n    args: []", reference: "goose 1.52.0 (probed 2026-09-30)", docs_url: "https://goose-docs.ai/docs/getting-started/using-extensions/", note: "probed: `goose mcp <SERVER>` only *runs* goose's bundled servers (`goose mcp add` errors), so there is no add subcommand; the YAML config is user-edited — no machine-writable shape.", },
    AgentDef { id: "openhands", display_name: "OpenHands", strategy: Strategy::CliDirect, cli: Some("openhands"), verify_cli: None, config: ConfigPath::Home(".openhands/mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::TransportStdio, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "openhands 1.16.0 (probed 2026-09-30)", docs_url: "https://docs.openhands.dev/openhands/usage/cli/mcp-servers", note: "probed: `--transport {http,sse,stdio}` is REQUIRED (the grammar is `<name> --transport <T> <target>`, target being a positional); the JSON config is detection only.", },
    AgentDef { id: "kimi", display_name: "Kimi Code", strategy: Strategy::ConfigFile, cli: None, verify_cli: None, config: ConfigPath::Home(".kimi-code/mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Plain, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "kimi 2.1.1 (probed)", docs_url: "https://moonshotai.github.io/kimi-code/en/customization/mcp", note: "probed: Kimi Code 2.1.1 has no `mcp` subcommand (`kimi mcp add` belonged to the archived legacy kimi-cli); write the config file.", },
    AgentDef { id: "droid", display_name: "Droid (Factory)", strategy: Strategy::CliDirect, cli: Some("droid"), verify_cli: None, config: ConfigPath::Home(".factory/mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::DroidType, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "droid 0.230.0 (probed)", docs_url: "https://docs.factory.ai/cli/configuration/mcp", note: "probed: `droid mcp add [--type <http|sse|stdio>] <name> <urlOrCommand...>` — the flag is `--type` (no `--transport` alias) and the command is a positional; the JSON config is detection only.", },
    AgentDef { id: "crush", display_name: "Crush", strategy: Strategy::ConfigFile, cli: None, verify_cli: None, config: ConfigPath::Home(".config/crush/crushrc"), shape: ConfigShape::Unknown, dialect: Dialect::Crush, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: true, remedy: "crush has no `mcp` shell subcommand (`crush mcp add` fails with Unknown command). Paste this into `~/.config/crush/crushrc`:\nmcp add slugaudit --command {binary}\n(or write the legacy `crush.json` with key `mcp`).", reference: "crush 0.97.1 (probed)", docs_url: "https://github.com/charmbracelet/crush", note: "probed: `crush mcp add` fails with \"Unknown command\" — `mcp add` is only a crushrc bash builtin (`--command` form), so no machine-writable shape is established and the config is user-edited.", },
    AgentDef { id: "q", display_name: "Amazon Q Developer", strategy: Strategy::CliDirect, cli: Some("q"), verify_cli: None, config: ConfigPath::Unverified, shape: ConfigShape::Unknown, dialect: Dialect::Q, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "q 1.19.7 (probed 2026-09-30)", docs_url: "https://docs.aws.amazon.com/amazonq/latest/qdeveloper-ug/command-line-mcp.html", note: "probed: `q mcp add --name X --command Y` (both flags required, no transport flag); the Debian `q` collision is `python3-q-text-as-data`, which also ships /usr/bin/q.", },
    AgentDef { id: "cursor", display_name: "Cursor IDE", strategy: Strategy::ConfigFile, cli: None, verify_cli: Some("agent"), config: ConfigPath::Home(".cursor/mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Plain, evidence: Evidence::VerifiedByProbe, file_fallback: false, manual_steps: false, remedy: "", reference: "cursor 2026.09.28-64d2043 (probed)", docs_url: "https://cursor.com/docs/cli/mcp", note: "probed: `agent mcp` has no `add` (only login/list/list-tools/enable/disable; its docs agree), so `agent mcp list` verifies but cannot register; write `.cursor/mcp.json` (binary `agent`, legacy alias `cursor-agent`).", },
    AgentDef { id: "windsurf", display_name: "Windsurf", strategy: Strategy::ConfigFile, cli: None, verify_cli: None, config: ConfigPath::Home(".codeium/windsurf/mcp_config.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Plain, evidence: Evidence::DocVerified, file_fallback: false, manual_steps: false, remedy: "", reference: "vendor docs, as of 2026-09-30", docs_url: "https://docs.devin.ai/windsurf/plugins/cascade/mcp", note: "docs: config-file only, no CLI.", },
    AgentDef { id: "pi", display_name: "Pi Agent", strategy: Strategy::CliDirect, cli: Some("pi"), verify_cli: None, config: ConfigPath::Home(".pi/agent/mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::DashSeparator, evidence: Evidence::VerifiedByProbe, file_fallback: true, manual_steps: false, remedy: "", reference: "pi 0.99.1 (probed)", docs_url: "https://pi.dev/docs/latest/mcp", note: "probed: `pi mcp add` is real; `.pi/agent/mcp.json` is documented too and serves as the file fallback.", },
    AgentDef { id: "omp", display_name: "Oh My Pi (omp)", strategy: Strategy::CliInteractiveOnly, cli: None, verify_cli: None, config: ConfigPath::Home(".omp/agent/mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Plain, evidence: Evidence::VerifiedFromSource, file_fallback: false, manual_steps: false, remedy: "", reference: "omp 18.12.0 source and schema (checked 2026-10-17)", docs_url: "https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md", note: "omp 18.12.0 has no `mcp` subcommand; its own `mcp-schema.json` names `~/.omp/agent/mcp.json` with a top-level `mcpServers` map, and the entry connect writes validates against that schema. Not run: loading the entry needs a configured model provider. The in-session `/mcp add` is the client's own add path; the installer writes the config file instead.", },
    AgentDef { id: "warp", display_name: "Warp Terminal", strategy: Strategy::ConfigFile, cli: Some("oz"), verify_cli: None, config: ConfigPath::Home(".warp/.mcp.json"), shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Plain, evidence: Evidence::DocVerified, file_fallback: false, manual_steps: false, remedy: "", reference: "vendor docs, as of 2026-09-30", docs_url: "https://docs.warp.dev/agents/capabilities/mcp/", note: "filename is `.mcp.json` (leading dot); the newer CLI exposes agent commands as `oz ... --mcp` (oz is `cli` for detection only).", },
    AgentDef { id: "claude-desktop", display_name: "Claude Desktop", strategy: Strategy::ConfigFile, cli: None, verify_cli: None, config: ConfigPath::PerOs { macos: "Library/Application Support/Claude/claude_desktop_config.json", windows: "%APPDATA%/Claude/claude_desktop_config.json", linux: ".config/Claude/claude_desktop_config.json", }, shape: ConfigShape::McpServersCommandArgs, dialect: Dialect::Plain, evidence: Evidence::DocVerified, file_fallback: false, manual_steps: false, remedy: "", reference: "vendor docs, as of 2026-09-30", docs_url: "https://modelcontextprotocol.io/docs/develop/connect-local-servers", note: "per-OS config; the Linux path is convention, not vendor-documented.", },
    AgentDef { id: "zed", display_name: "Zed Editor", strategy: Strategy::ConfigFile, cli: None, verify_cli: None, config: ConfigPath::Home(".config/zed/settings.json"), shape: ConfigShape::ZedContextServer, dialect: Dialect::Plain, evidence: Evidence::VerifiedFromSource, file_fallback: false, manual_steps: false, remedy: "", reference: "Zed source, as of 2026-09-30", docs_url: "https://zed.dev/docs/ai/mcp", note: "Zed's source: `command` is a string with a sibling `args` array (nested `{path,args}` is the legacy shape).", },
];

/// Looks up an agent definition by identifier or display name, supporting common aliases.
pub fn find_agent(name: &str) -> Option<AgentDef> {
    let n = name.trim().to_ascii_lowercase();
    let n = match n.as_str() {
        "claude-code" | "claude_code" => "claude",
        "antigravity" => "agy",
        "github-copilot" | "github_copilot" => "copilot",
        "oh-my-pi" | "oh_my_pi" => "omp",
        other => other,
    };
    AGENTS
        .iter()
        .copied()
        .find(|a| a.id == n || a.display_name.eq_ignore_ascii_case(n))
}

/// Returns all agents in [`AGENTS`] that appear to be installed on the machine.
pub fn detected_agents(home: Option<&Path>) -> Vec<AgentDef> {
    AGENTS
        .iter()
        .copied()
        .filter(|a| a.is_installed(home))
        .collect()
}

#[cfg(test)]
#[path = "connect_registry_tests.rs"]
mod registry_tests;
