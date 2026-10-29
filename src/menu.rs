//! Interactive setup menu — one command that walks a human through the
//! whole SlugAudit onboarding: install the binary, connect to an agent,
//! disconnect, get instructions for any other client, or start the server.

use crate::cli;
use crate::connect;
use crate::install;
use crate::menu_render::{other_agent_instructions, render_menu};
use crate::util::Style;

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
        cli::out(format_args!("{}", render_menu(&style)))?;
        let choice = next_choice(
            &mut crate::prompt::read_prompt_line,
            &mut std::io::stdout().lock(),
        )?;
        match choice {
            MenuChoice::Install => install_step()?,
            MenuChoice::Connect => connect_step()?,
            MenuChoice::Disconnect => disconnect_step()?,
            MenuChoice::OtherClient => other_client_step()?,
            MenuChoice::Serve => {
                if confirm_serve()? {
                    return Ok(true);
                }
            }
            MenuChoice::Exit => return Ok(false),
            // `next_choice` re-prompts until the answer is valid, so this is
            // never reached; redrawing the banner would be the wrong fallback.
            MenuChoice::Invalid => continue,
        }
        cli::out(format_args!("\n"))?;
    }
}

/// The line shown on a bad menu answer, followed by the choice prompt again.
const INVALID_CHOICE: &str =
    "Invalid choice; pick a number from the list.\nChoose an option [1-6]: ";

/// Reads menu answers until one is valid. A bad answer prints one error line
/// and re-asks on the choice line only; the banner is never redrawn.
fn next_choice(
    read: &mut impl FnMut() -> std::io::Result<Option<String>>,
    out: &mut impl std::io::Write,
) -> std::io::Result<MenuChoice> {
    loop {
        let choice = menu_choice(read()?.as_deref());
        if choice != MenuChoice::Invalid {
            return Ok(choice);
        }
        cli::write_to(out, format_args!("{INVALID_CHOICE}"))?;
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

fn install_step() -> Result<(), Box<dyn std::error::Error>> {
    cli::out(format_args!("\n"))?;
    match install::run_install() {
        Ok(()) => Ok(()),
        Err(error) => {
            cli::err(format_args!("Install failed: {error}\n"))?;
            Ok(())
        }
    }
}

fn connect_step() -> Result<(), Box<dyn std::error::Error>> {
    cli::out(format_args!("\n"))?;
    match connect::run_connect_interactive() {
        Ok(()) => Ok(()),
        Err(error) => {
            cli::err(format_args!("Connect failed: {error}\n"))?;
            Ok(())
        }
    }
}

fn disconnect_step() -> Result<(), Box<dyn std::error::Error>> {
    cli::out(format_args!("\n"))?;
    match connect::run_disconnect_interactive() {
        Ok(()) => Ok(()),
        Err(error) => {
            cli::err(format_args!("Disconnect failed: {error}\n"))?;
            Ok(())
        }
    }
}

fn other_client_step() -> Result<(), Box<dyn std::error::Error>> {
    cli::out(format_args!("\n"))?;
    let binary = match install::running_binary()
        .and_then(|current| connect::prefer_slugthug_binary(&current))
    {
        Ok(binary) => binary,
        Err(error) => {
            cli::err(format_args!(
                "could not locate the slugaudit binary: {error}\n"
            ))?;
            return Ok(());
        }
    };
    Ok(cli::out(format_args!(
        "{}",
        other_agent_instructions(&binary)
    ))?)
}

fn confirm_serve() -> Result<bool, Box<dyn std::error::Error>> {
    cli::out(format_args!(
        "\nThe MCP server (`serve`) is normally started automatically by your AI agent.\n\
         Start the server now? [y/N]: "
    ))?;
    // End of input declines: `[y/N]` defaults to no, and the menu loop exits on
    // the same end of input at its next read.
    Ok(crate::prompt::read_prompt_line()?.is_some_and(|line| crate::prompt::is_yes(&line)))
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
    fn an_invalid_answer_prints_one_error_line_and_reprompts_without_the_banner() {
        let mut answers = vec![
            Some("x".to_owned()),
            Some("9".to_owned()),
            Some("6".to_owned()),
        ]
        .into_iter();
        let mut out = Vec::new();

        let choice = next_choice(&mut || Ok(answers.next().flatten()), &mut out)
            .expect("reads and writes succeed");

        assert_eq!(choice, MenuChoice::Exit);
        let text = String::from_utf8(out).expect("utf8 output");
        assert_eq!(text.matches("Invalid choice;").count(), 2, "{text}");
        assert_eq!(
            text.matches("Choose an option [1-6]: ").count(),
            2,
            "{text}"
        );
        assert!(!text.contains("Install"), "no banner redraw: {text}");
        assert_eq!(text, INVALID_CHOICE.repeat(2));
        // The banner is rendered only by `run_menu`'s outer loop, once per menu.
        assert_eq!(
            render_menu(&Style::plain())
                .matches("Choose an option")
                .count(),
            1
        );
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
