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
        print!("{MENU}");
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
SlugAudit setup

  1) Install the binary (~/.slugthug/bin)
     A stable path for agents and MCP clients to launch.
  2) Connect to an AI agent
     Register this binary as the `slugaudit` MCP server in Claude Code,
     Grok, or Codex.
  3) Add SlugAudit to another MCP client
     Prints instructions + a config snippet for any other tool that
     supports MCP servers (Cursor, VS Code, Cline, Zed, ...).
  4) Run the MCP server now
     Advanced: starts `serve` in this terminal and blocks until Ctrl-C.
     Normally your AI agent starts the server for you.
  5) Exit

Choose an option [1-5]: ";

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
/// one of the three built-ins. SlugAudit is a standard stdio MCP server,
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
