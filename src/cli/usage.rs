//! The `--help` usage text and the styling that renders it.
//!
//! Every command is documented once, in a [`CommandDoc`]. The general help
//! ([`usage_text`]) and each subcommand's own help ([`topic_usage`]) are both
//! built from those entries, so they cannot drift apart. The parent re-exports
//! the public [`usage`] entry point, and imports [`usage_text`] and
//! [`render_usage`] for `cli_tests`.

use crate::util::Style;

/// One command's documentation: the single source for the general help and the
/// subcommand's own help.
struct CommandDoc {
    /// The command word.
    name: &'static str,
    /// The invocation after `slugaudit `, as the general help lists it.
    invocation: &'static str,
    /// Further invocations after `slugaudit `, shown in the subcommand's own
    /// help.
    aliases: &'static [&'static str],
    /// One line for the general help's `USAGE:` list.
    summary: &'static str,
    /// The description, one source line per output line, unindented.
    detail: &'static str,
}

const SERVE: CommandDoc = CommandDoc {
    name: "serve",
    invocation: "serve",
    aliases: &[],
    summary: "Run the MCP server (stdio transport)",
    detail: "Run the MCP server over stdio. This is also what `slugaudit` does with no\n\
arguments, and the way your AI agent launches SlugAudit. It speaks JSON-RPC\n\
over stdio and blocks until the host closes the pipe.",
};

const MENU: CommandDoc = CommandDoc {
    name: "menu",
    invocation: "menu",
    aliases: &[],
    summary: "Interactive setup menu (recommended entry point)",
    detail: "Open the interactive setup menu: install the binary, connect to an AI agent,\n\
get a config snippet for any other MCP client, or run the server for testing.\n\
Recommended for first-time setup.",
};

const INSTALL: CommandDoc = CommandDoc {
    name: "install",
    invocation: "install",
    aliases: &[],
    summary: "Copy binary to ~/.slugthug/slugaudit/",
    detail: "Copy the binary to ~/.slugthug/slugaudit/slugaudit so agents and MCP clients\n\
can launch a stable path that survives rebuilds.",
};

const CONNECT: CommandDoc = CommandDoc {
    name: "connect",
    invocation: "connect [AGENT]",
    aliases: &[],
    summary: "Connect to an AI agent or editor",
    detail: "Register the installed SlugAudit binary as the `slugaudit` MCP server in an\n\
AI agent or editor (or this binary when no installed copy exists). With an\n\
agent name (e.g. `agy`, `claude`, `gemini`, `cursor`) it connects directly;\n\
without one it shows the detected installed agents.",
};

const DISCONNECT: CommandDoc = CommandDoc {
    name: "disconnect",
    invocation: "disconnect [AGENT]",
    aliases: &["remove [AGENT]"],
    summary: "Disconnect from an AI agent or editor",
    detail: "Remove the SlugAudit registration from an AI agent or editor. Alias:\n\
`remove`. With an agent name it disconnects directly; without one it lists\n\
the connected agents to pick from.",
};

const UPDATE: CommandDoc = CommandDoc {
    name: "update",
    invocation: "update",
    aliases: &[],
    summary: "Fetch the latest release and replace this binary",
    detail: "Check the latest GitHub release and, if newer than the running binary,\n\
download it and atomically replace this binary in place. Uses curl; verifies\n\
the sha256 checksum before touching the current executable.",
};

const VERSION: CommandDoc = CommandDoc {
    name: "version",
    invocation: "version",
    aliases: &[],
    summary: "Print version (also --version, -V, -v)",
    detail: "Print the version. Also --version, -V, -v.",
};

const HELP: CommandDoc = CommandDoc {
    name: "help",
    invocation: "help",
    aliases: &[],
    summary: "Show this message (also --help, -h)",
    detail: "Show this message. Also --help, -h.",
};

/// Every command, in the order the general help lists them.
const COMMANDS: [&CommandDoc; 8] = [
    &MENU,
    &INSTALL,
    &CONNECT,
    &DISCONNECT,
    &UPDATE,
    &SERVE,
    &VERSION,
    &HELP,
];

/// The `EXAMPLES:` rows: invocation and what it does.
const EXAMPLES: [(&str, &str); 7] = [
    ("menu", "Set everything up interactively"),
    ("install", "Install to ~/.slugthug/slugaudit/"),
    ("connect agy", "Register with Antigravity"),
    ("connect claude", "Register with Claude Code"),
    ("disconnect claude", "Remove registration from Claude Code"),
    ("update", "Update to the latest release"),
    ("", "Run the server for your AI agent"),
];

/// Width of the invocation column in the general help.
const INVOCATION_WIDTH: usize = 23;

/// Width of the command-word column in the general help's `COMMANDS:` list.
const NAME_WIDTH: usize = 14;

/// The general usage and help text printed for `--help`, built from
/// [`COMMANDS`].
///
/// Crate-internal: [`usage`] is the public entry point, and nothing outside
/// this module reads the unstyled text.
pub(crate) fn usage_text() -> String {
    let mut lines = vec![
        "slugaudit — searchable, trustworthy codebase evidence over MCP".to_owned(),
        String::new(),
        "USAGE:".to_owned(),
        row("", "Run the MCP server (stdio transport)"),
    ];
    lines.extend(COMMANDS.iter().map(|doc| row(doc.invocation, doc.summary)));
    lines.push(String::new());
    lines.push("COMMANDS:".to_owned());
    for doc in COMMANDS {
        for (index, line) in doc.detail.lines().enumerate() {
            let name = if index == 0 { doc.name } else { "" };
            lines.push(format!("    {name:<NAME_WIDTH$}{line}"));
        }
    }
    lines.push(String::new());
    lines.push("OPTIONS:".to_owned());
    lines.push("    -h, --help      Show this message.".to_owned());
    lines.push("    -V, --version   Print the version.".to_owned());
    lines.push(String::new());
    lines.push("EXAMPLES:".to_owned());
    lines.extend(EXAMPLES.iter().map(|(command, what)| row(command, what)));
    lines.join("\n") + "\n"
}

/// One `    slugaudit <invocation>  <text>` row.
fn row(invocation: &str, text: &str) -> String {
    format!("    slugaudit {invocation:<INVOCATION_WIDTH$}{text}")
}

/// Formats the usage text, applying ANSI terminal styling when stdout is a terminal.
#[must_use]
pub fn usage() -> String {
    render_usage(&Style::stdout())
}

/// Renders [`usage_text`] with `style`. Split from [`usage`] so both branches can be
/// driven with a fixed style instead of whatever stdout happens to be.
pub(super) fn render_usage(style: &Style) -> String {
    if !style.enabled() {
        return usage_text();
    }
    let headings = ["USAGE:", "COMMANDS:", "OPTIONS:", "EXAMPLES:"];
    let text = usage_text();
    let mut out = String::with_capacity(text.len() + 64);
    for (i, line) in text.lines().enumerate() {
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

    /// The documentation entry for this subcommand.
    const fn doc(self) -> &'static CommandDoc {
        match self {
            Self::Serve => &SERVE,
            Self::Connect => &CONNECT,
            Self::Disconnect => &DISCONNECT,
            Self::Install => &INSTALL,
            Self::Menu => &MENU,
            Self::Update => &UPDATE,
        }
    }

    /// The usage text for this subcommand, built from its [`CommandDoc`].
    fn text(self) -> String {
        let doc = self.doc();
        let invocations = std::iter::once(doc.invocation).chain(doc.aliases.iter().copied());
        let mut lines = vec!["USAGE:".to_owned()];
        lines.extend(invocations.map(|invocation| format!("    slugaudit {invocation}")));
        lines.push(String::new());
        lines.push(doc.detail.to_owned());
        lines.push(String::new());
        lines.push("OPTIONS:".to_owned());
        lines.push("    -h, --help      Show this message.".to_owned());
        lines.join("\n") + "\n"
    }
}

/// The usage text for one subcommand, printed for `<subcommand> --help`.
#[must_use]
pub fn topic_usage(topic: Topic) -> String {
    topic.text()
}
