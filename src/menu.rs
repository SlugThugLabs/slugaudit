//! Interactive setup menu — one command that walks a human through the
//! whole SlugAudit onboarding: install the binary, connect to an agent,
//! disconnect, get instructions for any other client, or start the server.
#![allow(clippy::print_stdout)]

use crate::connect;
use crate::install;
use crate::menu_render::{other_agent_instructions, render_menu};
use crate::util::Style;
use std::io::Write as _;

/// Interactive setup menu. Returns `Ok(true)` when the user chooses to start
/// the MCP server, `Ok(false)` to exit without serving; input problems are
/// reported by the menu itself.
///
/// # Errors
///
/// Returns an error only when reading the menu's stdin fails outright.
pub fn run_menu() -> Result<bool, Box<dyn std::error::Error>> {
    loop {
        let style = Style::stdout();
        print!("{}", render_menu(&style));
        std::io::stdout().flush()?;
        let line = crate::prompt::read_prompt_line()?;
        match menu_choice(line.as_deref()) {
            MenuChoice::Install => install_step(),
            MenuChoice::Connect => connect_step(),
            MenuChoice::Disconnect => disconnect_step(),
            MenuChoice::OtherClient => other_client_step(),
            MenuChoice::Serve => {
                if confirm_serve()? {
                    return Ok(true);
                }
            }
            MenuChoice::Exit => return Ok(false),
            MenuChoice::Invalid => {
                println!("\nInvalid choice; pick a number from the list.");
                continue;
            }
        }
        println!();
    }
}

/// One menu answer, decided apart from the terminal that read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuChoice {
    Install,
    Connect,
    Disconnect,
    OtherClient,
    Serve,
    Exit,
    Invalid,
}

/// Maps a menu line to its choice. End of input (`None`) exits, so a closed
/// stdin can never spin the menu loop.
fn menu_choice(line: Option<&str>) -> MenuChoice {
    let Some(line) = line else {
        return MenuChoice::Exit;
    };
    match line.trim().parse::<usize>() {
        Ok(1) => MenuChoice::Install,
        Ok(2) => MenuChoice::Connect,
        Ok(3) => MenuChoice::Disconnect,
        Ok(4) => MenuChoice::OtherClient,
        Ok(5) => MenuChoice::Serve,
        Ok(6) => MenuChoice::Exit,
        _ => MenuChoice::Invalid,
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
    if let Err(error) = std::io::stdout().flush() {
        eprintln!("could not flush instructions to stdout: {error}");
    }
}

fn confirm_serve() -> Result<bool, Box<dyn std::error::Error>> {
    println!();
    println!("The MCP server (`serve`) is normally started automatically by your AI agent.");
    print!("Start the server now? [y/N]: ");
    std::io::stdout().flush()?;
    let line = crate::prompt::read_prompt_line()?.unwrap_or_default();
    Ok(crate::prompt::is_yes(&line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu_render::{MENU, section_label};
    use std::path::Path;

    #[test]
    fn each_listed_number_selects_its_step_and_end_of_input_exits() {
        let cases = [
            (Some("1"), MenuChoice::Install),
            (Some(" 2\n"), MenuChoice::Connect),
            (Some("3"), MenuChoice::Disconnect),
            (Some("4"), MenuChoice::OtherClient),
            (Some("5"), MenuChoice::Serve),
            (Some("6"), MenuChoice::Exit),
            (None, MenuChoice::Exit),
        ];

        for (line, expected) in cases {
            assert_eq!(menu_choice(line), expected, "line {line:?}");
        }
    }

    #[test]
    fn an_unlisted_or_non_numeric_menu_answer_is_invalid() {
        let answers = ["0", "7", "", "install", "-1"];

        for answer in answers {
            assert_eq!(menu_choice(Some(answer)), MenuChoice::Invalid, "{answer:?}");
        }
    }

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
