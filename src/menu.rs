//! Interactive setup menu — one command that walks a human through the
//! whole SlugAudit onboarding: install the binary, connect to a supported
//! AI agent, get instructions for adding SlugAudit to any other
//! MCP-capable client, or start the server directly. Exposed as
//! `slugaudit-mcp menu`.
//!
//! This is a human-facing CLI (never the serve path), so stdout is fine —
//! same exception as connect.rs/install.rs. The menu is deliberately a
//! thin driver over the existing `install`/`connect` entry points and
//! owns no setup logic of its own, so the non-interactive commands stay
//! the single source of truth for what each step does.
#![allow(clippy::print_stdout)]

use crate::connect;
use crate::install;
use crate::util::Style;
use std::io::{BufRead as _, Write as _};
use std::path::Path;

/// Runs the interactive setup menu. Returns `Ok(true)` if the user chose
/// to start the MCP server from the menu (the caller should fall through
/// to `serve`), `Ok(false)` if the menu exited without starting it.
///
/// # Errors
///
/// Returns an error only if reading menu input fails.
pub fn run_menu() -> Result<bool, Box<dyn std::error::Error>> {
    loop {
        let style = Style::stdout();
        print!("{}", render_menu(&style));
        std::io::stdout().flush()?;
        let choice = read_choice()?;
        match choice {
            1 => install_step(),
            2 => connect_step(),
            3 => other_client_step(),
            4 => {
                if confirm_serve()? {
                    return Ok(true);
                }
            }
            5 => return Ok(false),
            _ => {
                println!("\nInvalid choice; pick a number from the list.");
                continue;
            }
        }
        println!();
    }
}

const MENU: &str = "\
┌─────────────────────────────────────────────┐
│            SlugAudit setup                  │
└─────────────────────────────────────────────┘

  ── Setup ───────────────────────────────────

  1) Install the binary  (~/.slugthug/bin)
     A stable path for agents and MCP clients to launch.

  2) Connect to an AI agent
     Register this binary as the `slugaudit` MCP server in
     Bob, Claude Code, Grok, or Codex.

  3) Add SlugAudit to another MCP client
     Print instructions + a config snippet for any other
     tool that supports MCP servers (Cursor, VS Code,
     Cline, Zed, ...).

  ── Advanced ─────────────────────────────────

  4) Run the MCP server now
     Advanced: starts `serve` in this terminal and blocks
     until Ctrl-C. Normally your AI agent starts it for you.

  ── Exit ─────────────────────────────────────

  5) Exit

Choose an option [1-5]: ";

/// Renders the menu, applying color only when stdout is a real terminal.
/// When stdout is piped or captured (`Style::plain`), the returned string
/// is byte-for-byte the plain `MENU` text, so redirects, CI logs, and the
/// test harness never see escape sequences. When it is a terminal, the
/// title bar, section rules, and prompt are highlighted so the layout the
/// human sees reads faster without changing any content or alignment.
fn render_menu(style: &Style) -> String {
    if !style.enabled() {
        return MENU.to_owned();
    }
    // The trailing dash runs differ per rule line, so we locate the `── Label ─`
    // prefix by its position and re-use the existing leading `──`s. Each
    // recognized line keeps its own indentation and dash count.
    let mut out = String::with_capacity(MENU.len() + 96);

    for line in MENU.lines() {
        let trimmed = line.trim();
        if let Some(label) = section_label(trimmed) {
            // Rebuild `── Label ─` in green with the label bold, then dim
            // the remaining dashes (everything after the label's closing `─`).
            let prefix_len = line.find(label).unwrap_or(0);
            let before = &line[..prefix_len];
            let after = &line[prefix_len + label.len()..];
            out.push_str(before);
            out.push_str(&style.green(&style.bold(label)));
            out.push_str(&style.dim(after));
        } else if trimmed.starts_with('┌') || trimmed.starts_with('└') {
            out.push_str(&style.dim(line));
        } else if trimmed.contains("SlugAudit setup") {
            out.push_str(&line.replace(
                "SlugAudit setup",
                &style.bold(&style.cyan("SlugAudit setup")),
            ));
        } else if trimmed.starts_with("Choose an option") {
            out.push_str(&style.bold(&style.yellow(line)));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    out
}

/// Returns the bare section label ("Setup", "Advanced", "Exit") when a
/// line is a `── Label ─……` rule. `trimmed` has leading/trailing
/// whitespace removed. The label sits between a leading `── ` and the
/// ` ─` before the dash run.
fn section_label(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("── ")?;
    let (label, _after) = rest.split_once(" ─")?;
    if label.is_empty() || !label.chars().all(|c| c.is_alphanumeric() || c == ' ') {
        return None;
    }
    Some(label)
}

/// Reads one menu choice from stdin. Unparseable input maps to `0` so the
/// caller's `_` arm reports it as invalid rather than panicking.
fn read_choice() -> Result<usize, Box<dyn std::error::Error>> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().parse().unwrap_or(0))
}

/// Runs the install step, surfacing failures inline so the menu survives a
/// bad HOME / SLUGTHUG_HOME instead of aborting the whole session.
fn install_step() {
    println!();
    match install::run_install() {
        Ok(()) => {}
        Err(error) => eprintln!("Install failed: {error}"),
    }
}

/// Runs the existing interactive connect flow (its own agent menu).
fn connect_step() {
    println!();
    match connect::run_connect_interactive() {
        Ok(()) => {}
        Err(error) => eprintln!("Connect failed: {error}"),
    }
}

/// Prints instructions for adding SlugAudit to an MCP client that isn't
/// one of the four built-ins. SlugAudit is a standard stdio MCP server,
/// so any MCP-capable client can add it by name + command — the client's
/// own docs describe where its MCP config lives; we supply the entry.
fn other_client_step() {
    println!();
    let binary = install::running_binary()
        .map(|current| connect::prefer_slugthug_binary(&current))
        .unwrap_or_else(|_| std::path::PathBuf::from("slugaudit-mcp"));
    print!("{}", other_agent_instructions(&binary));
    std::io::stdout().flush().ok();
}

/// Builds the "add SlugAudit to any MCP client" instructions for a given
/// binary path. Pure so it can be unit-tested without stdin/stdout.
fn other_agent_instructions(binary: &Path) -> String {
    let mut text = String::new();
    text.push_str(
        "SlugAudit is a standard stdio MCP server, so any MCP-capable client can add it:\n\n",
    );
    text.push_str("  Server name:  slugaudit\n");
    text.push_str(&format!("  Command:      {}\n", binary.display()));
    text.push_str("  Arguments:    (none - `serve` is the default)\n\n");
    text.push_str(
        "Most clients that read a standard MCP config use a file like `.mcp.json`\n\
         (project scope) or a user-level equivalent; the entry looks like:\n\n",
    );
    text.push_str("{\n");
    text.push_str("  \"mcpServers\": {\n");
    text.push_str("    \"slugaudit\": {\n");
    text.push_str(&format!("      \"command\": \"{}\",\n", binary.display()));
    text.push_str("      \"args\": []\n");
    text.push_str("    }\n");
    text.push_str("  }\n");
    text.push_str("}\n\n");
    text.push_str(
        "Check your client's own documentation for where it reads MCP server\n\
         config, and paste the entry above (adjusting the command path if you\n\
         moved the binary). If you have run `slugaudit-mcp install`, the stable\n\
         command path is ~/.slugthug/bin/slugaudit-mcp\n",
    );
    text
}

/// Asks before starting `serve`, since it blocks the terminal and is
/// normally launched by the agent rather than by hand.
fn confirm_serve() -> Result<bool, Box<dyn std::error::Error>> {
    println!();
    println!(
        "The MCP server (`serve`) is normally started automatically by your AI agent.\n\
         Running it here starts it in this terminal and blocks until you press Ctrl-C -\n\
         useful for testing by hand, or for a custom MCP client that launches it itself."
    );
    print!("Start the server now? [y/N]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_style_renders_the_byte_exact_plain_menu() {
        let plain = render_menu(&Style::plain());
        assert_eq!(plain, MENU, "plain rendering must match the menu const exactly");
        assert!(
            !plain.contains('\x1b'),
            "plain rendering must never contain ESC escape bytes"
        );
    }

    #[test]
    fn section_label_extracts_the_rule_label_and_nothing_else() {
        assert_eq!(section_label("── Setup ──"), Some("Setup"));
        assert_eq!(section_label("── Advanced ──"), Some("Advanced"));
        assert_eq!(section_label("── Exit ────"), Some("Exit"));
        assert_eq!(section_label("not a rule"), None);
        assert_eq!(section_label("──"), None);
        assert!(section_label("── Setup ──").is_some());
    }

    #[test]
    fn other_agent_instructions_name_the_server_and_command() {
        let text = other_agent_instructions(Path::new("/opt/slugaudit-mcp"));
        assert!(text.contains("slugaudit"));
        assert!(text.contains("/opt/slugaudit-mcp"));
        assert!(text.contains("\"command\""));
        assert!(text.contains("args"));
    }

    #[test]
    fn other_agent_instructions_mention_the_stable_install_path() {
        let text = other_agent_instructions(Path::new("slugaudit-mcp"));
        assert!(text.contains(".slugthug/bin"));
    }
}
