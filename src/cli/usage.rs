//! The `--help` usage text and the styling that renders it.
//!
//! Split from the parent module to keep `cli.rs` under the source-size gate.
//! The parent re-exports the public [`usage`] entry point, and imports
//! [`USAGE`] and [`render_usage`] for `cli_tests`; their crate paths are
//! otherwise unchanged.

use crate::util::Style;

/// The raw usage and help text printed for `--help`.
///
/// Crate-internal: [`usage`] is the public entry point, and nothing outside
/// this module reads the unstyled text.
pub(crate) const USAGE: &str = "\
slugaudit — searchable, trustworthy codebase evidence over MCP

USAGE:
    slugaudit                        Run the MCP server (stdio transport)
    slugaudit menu                   Interactive setup menu (recommended entry point)
    slugaudit install                Copy binary to ~/.slugthug/slugaudit/
    slugaudit connect [AGENT]        Connect to an AI agent or editor
    slugaudit disconnect [AGENT]     Disconnect from an AI agent or editor
    slugaudit update                 Fetch the latest release and replace this binary
    slugaudit version                Print version (also --version, -V, -v)
    slugaudit help                   Show this message (also --help, -h)

COMMANDS:
    (no command)  Start the MCP server. This is the default and the way
                  your AI agent launches SlugAudit; it speaks JSON-RPC
                  over stdio and blocks until the host closes the pipe.
    menu          Interactive setup: install the binary, connect to an AI
                  agent, get a config snippet for any other MCP client,
                  or run the server for testing. Recommended for first-time setup.
    connect       Register the installed SlugAudit binary as the `slugaudit`
                  MCP server in an AI agent or editor (or this binary when no
                  installed copy exists). With an agent name (e.g. `agy`,
                  `claude`, `gemini`, `cursor`) it connects directly; without
                  one it shows detected installed agents.
    disconnect    Remove SlugAudit registration from an AI agent or editor.
                  Alias: `remove`. With an agent name it disconnects directly;
                  without one it lists connected agents to pick from.
    install       Copy the binary to ~/.slugthug/slugaudit/slugaudit so
                  agents and MCP clients can launch a stable path that
                  survives rebuilds.
    update        Check the latest GitHub release and, if newer than the
                  running binary, download it and atomically replace this
                  binary in place. Uses curl; verifies the sha256 checksum
                  before touching the current executable.
    version       Print the version. Also --version, -V, -v.
    help          Show this message. Also --help, -h.

OPTIONS:
    -h, --help      Show this message.
    -V, --version   Print the version.

EXAMPLES:
    slugaudit menu                   Set everything up interactively
    slugaudit install                Install to ~/.slugthug/slugaudit/
    slugaudit connect agy            Register with Antigravity
    slugaudit connect claude         Register with Claude Code
    slugaudit disconnect claude      Remove registration from Claude Code
    slugaudit update                 Update to the latest release
    slugaudit                        Run the server for your AI agent
";

/// Formats the usage text, applying ANSI terminal styling when stdout is a terminal.
pub fn usage() -> String {
    render_usage(&Style::stdout())
}

/// Renders [`USAGE`] with `style`. Split from [`usage`] so both branches can be
/// driven with a fixed style instead of whatever stdout happens to be.
pub(super) fn render_usage(style: &Style) -> String {
    if !style.enabled() {
        return USAGE.to_owned();
    }
    let headings = ["USAGE:", "COMMANDS:", "OPTIONS:", "EXAMPLES:"];
    let mut out = String::with_capacity(USAGE.len() + 64);
    for (i, line) in USAGE.lines().enumerate() {
        let trimmed = line.trim_end();
        if i == 0 {
            out.push_str(&style.bold(trimmed));
        } else if headings.contains(&trimmed) {
            out.push_str(&style.bold(&style.cyan(trimmed)));
        } else {
            out.push_str(trimmed);
        }
        out.push('\n');
    }
    out
}

/// A subcommand that has its own `--help` text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    /// `slugaudit serve`.
    Serve,
    /// `slugaudit connect`.
    Connect,
    /// `slugaudit disconnect` (alias `remove`).
    Disconnect,
    /// `slugaudit install`.
    Install,
    /// `slugaudit menu`.
    Menu,
    /// `slugaudit update`.
    Update,
}

impl Topic {
    /// The topic for a subcommand name, or `None` for a name without one.
    pub(super) fn for_command(name: &str) -> Option<Self> {
        match name {
            "serve" => Some(Self::Serve),
            "connect" => Some(Self::Connect),
            "disconnect" | "remove" => Some(Self::Disconnect),
            "install" => Some(Self::Install),
            "menu" => Some(Self::Menu),
            "update" => Some(Self::Update),
            _ => None,
        }
    }

    /// The usage text for this subcommand.
    const fn text(self) -> &'static str {
        match self {
            Self::Serve => {
                "\
USAGE:
    slugaudit serve

Run the MCP server over stdio. This is also what `slugaudit` does with no
arguments. It blocks until the host closes the pipe.

OPTIONS:
    -h, --help      Show this message.
"
            }
            Self::Connect => {
                "\
USAGE:
    slugaudit connect [AGENT]

Register SlugAudit as the `slugaudit` MCP server in an AI agent or editor.
With an agent name (e.g. `agy`, `claude`, `gemini`, `cursor`) it connects
directly; without one it shows the detected installed agents.

OPTIONS:
    -h, --help      Show this message.
"
            }
            Self::Disconnect => {
                "\
USAGE:
    slugaudit disconnect [AGENT]
    slugaudit remove [AGENT]

Remove the SlugAudit registration from an AI agent or editor. With an agent
name it disconnects directly; without one it lists connected agents.

OPTIONS:
    -h, --help      Show this message.
"
            }
            Self::Install => {
                "\
USAGE:
    slugaudit install

Copy this binary to ~/.slugthug/slugaudit/slugaudit so agents can launch a
stable path that survives rebuilds.

OPTIONS:
    -h, --help      Show this message.
"
            }
            Self::Menu => {
                "\
USAGE:
    slugaudit menu

Open the interactive setup menu: install the binary, connect to an AI agent,
get a config snippet for another MCP client, or run the server for testing.

OPTIONS:
    -h, --help      Show this message.
"
            }
            Self::Update => {
                "\
USAGE:
    slugaudit update

Check the latest GitHub release and, if newer than the running binary,
download it, verify its sha256 checksum, and replace this binary in place.

OPTIONS:
    -h, --help      Show this message.
"
            }
        }
    }
}

/// The usage text for one subcommand, printed for `<subcommand> --help`.
#[must_use]
pub fn topic_usage(topic: Topic) -> &'static str {
    topic.text()
}
