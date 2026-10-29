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
