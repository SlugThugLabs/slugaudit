//! Interactive setup menu — one command that walks a human through the
//! whole SlugAudit onboarding: install the binary, connect to an agent,
//! disconnect, get instructions for any other client, or start the server.
#![allow(clippy::print_stdout)]

use crate::connect;
use crate::install;
use crate::menu_render::{other_agent_instructions, render_menu};
use crate::util::Style;
use std::io::{BufRead as _, Write as _};

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
    use crate::menu_render::{MENU, section_label};
    use std::path::Path;

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
