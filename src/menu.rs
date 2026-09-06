//! Interactive setup menu — one command that walks a human through the
//! whole SlugAudit onboarding: install the binary, connect to an agent,
//! disconnect, get instructions for any other client, or start the server.
#![allow(clippy::print_stdout)]
// slugaudit-line-exception: approved-by=slugthug; reason=the interactive setup menu keeps rendering, choice dispatch, and its setup branches together so the user-facing flow remains one cohesive CLI contract

use crate::connect;
use crate::install;
use crate::util::Style;
use std::io::{BufRead as _, Write as _};
use std::path::Path;

pub fn run_menu() -> Result<bool, Box<dyn std::error::Error>> {
    loop {
        let style = Style::stdout();
        print!("{}", render_menu(&style));
        std::io::stdout().flush()?;
        let choice = read_choice()?;
        match choice {
            1 => install_step(),
            2 => connect_step(),
            3 => disconnect_step(),
            4 => other_client_step(),
            5 => {
                if confirm_serve()? {
                    return Ok(true);
                }
            }
            6 => return Ok(false),
            _ => {
                println!("\nInvalid choice; pick a number from the list.");
                continue;
            }
        }
        println!();
    }
}

const MENU: &str = "  ____  _             _             _ _ _   
 / ___|| |_   _  __ _/ \\  _   _  __| (_) |_ 
 \\___ \\| | | | |/ _` / _ \\| | | |/ _` | | __|
  ___) | | |_| | (_| / ___ \\ |_| | (_| | | |_ 
 |____/|_|\\__,_|\\__, /_/   \\_\\__,_|\\__,_|_|\\__|
                |___/                          
┌─────────────────────────────────────────────┐
│            SlugAudit setup                  │
└─────────────────────────────────────────────┘

  ── Setup ───────────────────────────────────

  1) Install the binary  (~/.slugthug/slugaudit)
     A stable path for agents and MCP clients to launch.

  2) Connect to an AI agent
     Auto-detect installed agents and register SlugAudit.

  3) Disconnect from an AI agent
     Remove SlugAudit registration from an agent or editor.

  4) Add SlugAudit to another MCP client
     Print instructions + a config snippet for any other
     tool that supports MCP servers (Cursor, VS Code, ...).

  ── Advanced ─────────────────────────────────

  5) Run the MCP server now
     Advanced: starts `serve` in this terminal and blocks
     until Ctrl-C. Normally your AI agent starts it for you.

  ── Exit ─────────────────────────────────────

  6) Exit

Choose an option [1-6]: ";

fn render_menu(style: &Style) -> String {
    if !style.enabled() {
        return MENU.to_owned();
    }
    let mut out = String::with_capacity(MENU.len() + 96);
    for line in MENU.lines() {
        let trimmed = line.trim();
        if let Some(label) = section_label(trimmed) {
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

fn section_label(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("── ")?;
    let (label, _after) = rest.split_once(" ─")?;
    if label.is_empty() || !label.chars().all(|c| c.is_alphanumeric() || c == ' ') {
        return None;
    }
    Some(label)
}

fn read_choice() -> Result<usize, Box<dyn std::error::Error>> {
    let mut line = String::new();
    let bytes_read = std::io::stdin().lock().read_line(&mut line)?;
    if bytes_read == 0 {
        return Ok(6);
    }
    match line.trim().parse::<usize>() {
        Ok(choice) => Ok(choice),
        Err(_) => Ok(0),
    }
}

fn install_step() {
    println!();
    match install::run_install() {
        Ok(()) => {}
        Err(error) => eprintln!("Install failed: {error}"),
    }
}

fn connect_step() {
    println!();
    match connect::run_connect_interactive() {
        Ok(()) => {}
        Err(error) => eprintln!("Connect failed: {error}"),
    }
}

fn disconnect_step() {
    println!();
    match connect::run_disconnect_interactive() {
        Ok(()) => {}
        Err(error) => eprintln!("Disconnect failed: {error}"),
    }
}

fn other_client_step() {
    println!();
    let binary = install::running_binary()
        .map(|current| connect::prefer_slugthug_binary(&current))
        .unwrap_or_else(|_| std::path::PathBuf::from("slugaudit"));
    print!("{}", other_agent_instructions(&binary));
    std::io::stdout().flush().ok();
}

fn other_agent_instructions(binary: &Path) -> String {
    let mut text = String::new();
    text.push_str("SlugAudit is a standard stdio MCP server, so any MCP client can add it:\n\n");
    text.push_str("  Server name:  slugaudit\n");
    text.push_str(&format!("  Command:      {}\n", binary.display()));
    text.push_str("  Arguments:    (none - `serve` is the default)\n\n");
    text.push_str("Config entry for `.mcp.json` or equivalent:\n\n");
    text.push_str("{\n  \"mcpServers\": {\n    \"slugaudit\": {\n");
    text.push_str(&format!("      \"command\": \"{}\",\n", binary.display()));
    text.push_str("      \"args\": []\n    }\n  }\n}\n\n");
    text.push_str("Stable command path after install: ~/.slugthug/slugaudit/slugaudit\n");
    text
}

fn confirm_serve() -> Result<bool, Box<dyn std::error::Error>> {
    println!();
    println!("The MCP server (`serve`) is normally started automatically by your AI agent.");
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
        assert_eq!(plain, MENU);
        assert!(!plain.contains('\x1b'));
    }

    #[test]
    fn colored_style_renders_ansi_escapes() {
        let colored = render_menu(&Style::colored());
        assert!(colored.contains('\x1b'));
        assert!(colored.contains("SlugAudit setup"));
    }

    #[test]
    fn section_label_extracts_the_rule_label_and_nothing_else() {
        assert_eq!(section_label("── Setup ──"), Some("Setup"));
        assert_eq!(section_label("── Advanced ──"), Some("Advanced"));
        assert_eq!(section_label("── Exit ────"), Some("Exit"));
        assert_eq!(section_label("not a rule"), None);
    }

    #[test]
    fn other_agent_instructions_name_the_server_and_command() {
        let text = other_agent_instructions(Path::new("/opt/slugaudit"));
        assert!(text.contains("slugaudit"));
        assert!(text.contains("/opt/slugaudit"));
        assert!(text.contains("\"command\""));
        assert!(text.contains("args"));
    }

    #[test]
    fn other_agent_instructions_mention_the_stable_install_path() {
        let text = other_agent_instructions(Path::new("slugaudit"));
        assert!(text.contains(".slugthug/slugaudit/slugaudit"));
    }
}
