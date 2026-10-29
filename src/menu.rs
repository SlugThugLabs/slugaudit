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
#[path = "menu_tests.rs"]
mod tests;
