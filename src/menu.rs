// slugaudit-line-exception: approved-by=split-judge; reason=Single interactive setup menu concern (41-line MENU banner/options, ANSI styling, choice loop, and step dispatch); splitting would sever coupled 6-option layout and handlers.
//! Interactive setup menu: install the binary, connect or disconnect an AI
//! agent, print instructions for another MCP client, or start the server.

use crate::cli;
use crate::connect;
use crate::install;
use crate::util::Style;

macro_rules! choice_prompt {
    () => {
        "Choose an option [1-6]: "
    };
}

macro_rules! menu_title {
    () => {
        "SlugAudit setup"
    };
}

const CHOICE_PROMPT: &str = choice_prompt!();
const MENU_TITLE: &str = menu_title!();

const MENU: &str = concat!(
    "  ____  _             _             _ _ _   
 / ___|| |_   _  __ _/ \\  _   _  __| (_) |_ 
 \\___ \\| | | | |/ _` / _ \\| | | |/ _` | | __|
  ___) | | |_| | (_| / ___ \\ |_| | (_| | | |_ 
 |____/|_|\\__,_|\\__, /_/   \\_\\__,_|\\__,_|_|\\__|
                |___/                          
┌─────────────────────────────────────────────┐
│            ",
    menu_title!(),
    "                  │
└─────────────────────────────────────────────┘

  ── Setup ───────────────────────────────────

  1) Install the binary
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

",
    choice_prompt!()
);

/// Interactive setup menu. Returns `Ok(true)` when the user chooses to start
/// the MCP server, `Ok(false)` to exit without serving.
///
/// # Errors
///
/// Returns an error only when reading stdin fails outright.
pub fn run_menu() -> Result<bool, Box<dyn std::error::Error>> {
    loop {
        let style = Style::stdout();
        cli::out(format_args!("{}", render_menu(&style)))?;
        let max_invalid = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            None
        } else {
            Some(MAX_NON_INTERACTIVE_INVALID_ATTEMPTS)
        };
        let choice = next_choice_with_limit(
            &mut crate::prompt::read_prompt_line,
            &mut std::io::stdout().lock(),
            max_invalid,
        )?;
        if choice == MenuChoice::Exit {
            return Ok(false);
        }
        cli::out(format_args!("\n"))?;
        match choice {
            MenuChoice::Install => {
                try_step("Install failed", install::run_install())?;
            }
            MenuChoice::Connect => {
                try_step("Connect failed", connect::run_connect_interactive())?;
            }
            MenuChoice::Disconnect => {
                try_step("Disconnect failed", connect::run_disconnect_interactive())?;
            }
            MenuChoice::OtherClient => other_client_step()?,
            MenuChoice::Serve => {
                if confirm_serve()? {
                    return Ok(true);
                }
            }
            MenuChoice::Exit => return Ok(false),
        }
        cli::out(format_args!("\n"))?;
    }
}

const INVALID_CHOICE: &str = "Invalid choice; pick a number from the list.\n";

const MAX_NON_INTERACTIVE_INVALID_ATTEMPTS: usize = 16;

fn next_choice_with_limit(
    read: &mut impl FnMut() -> std::io::Result<Option<String>>,
    out: &mut impl std::io::Write,
    max_invalid: Option<usize>,
) -> std::io::Result<MenuChoice> {
    let mut invalid_attempts: usize = 0;
    loop {
        if let Some(choice) = menu_choice(read()?.as_deref()) {
            return Ok(choice);
        }
        // KEEP saturating: invalid_attempts is only compared against max_invalid; clamping at usize::MAX never resets below any limit
        invalid_attempts = invalid_attempts.saturating_add(1);
        if max_invalid.is_some_and(|limit| invalid_attempts >= limit) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "too many invalid menu choices on non-interactive input",
            ));
        }
        cli::write_to(out, format_args!("{INVALID_CHOICE}{CHOICE_PROMPT}"))?;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuChoice {
    Install,
    Connect,
    Disconnect,
    OtherClient,
    Serve,
    Exit,
}

fn menu_choice(line: Option<&str>) -> Option<MenuChoice> {
    let Some(line) = line else {
        return Some(MenuChoice::Exit);
    };
    match line.trim().parse::<usize>() {
        Ok(1) => Some(MenuChoice::Install),
        Ok(2) => Some(MenuChoice::Connect),
        Ok(3) => Some(MenuChoice::Disconnect),
        Ok(4) => Some(MenuChoice::OtherClient),
        Ok(5) => Some(MenuChoice::Serve),
        Ok(6) => Some(MenuChoice::Exit),
        _ => None,
    }
}

fn render_menu(style: &Style) -> String {
    if !style.enabled() {
        return MENU.to_owned();
    }
    let mut out = String::with_capacity(MENU.len());
    out.reserve(96);
    let mut lines = MENU.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if let Some(label) = section_label(trimmed) {
            let parts = line.find(label).and_then(|prefix_len| {
                let (before, rest) = line.split_at_checked(prefix_len)?;
                let (_label, after) = rest.split_at_checked(label.len())?;
                Some((before, after))
            });
            if let Some((before, after)) = parts {
                out.push_str(before);
                out.push_str(&style.green(&style.bold(label)));
                out.push_str(&style.dim(after));
            } else {
                out.push_str(line);
            }
        } else if trimmed.starts_with('┌') || trimmed.starts_with('└') {
            out.push_str(&style.dim(line));
        } else if trimmed.contains(MENU_TITLE) {
            out.push_str(&line.replace(MENU_TITLE, &style.bold(&style.cyan(MENU_TITLE))));
        } else if trimmed.starts_with(CHOICE_PROMPT.trim_end()) {
            out.push_str(&style.bold(&style.yellow(line)));
        } else {
            out.push_str(line);
        }
        if lines.peek().is_some() {
            out.push('\n');
        }
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

fn try_step<T, E: std::fmt::Display>(
    label: &str,
    result: Result<T, E>,
) -> Result<Option<T>, Box<dyn std::error::Error>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            cli::err(format_args!("{label}: {error}\n"))?;
            Ok(None)
        }
    }
}

fn other_client_step() -> Result<(), Box<dyn std::error::Error>> {
    let Some(binary) = try_step(
        "could not locate the slugaudit binary",
        install::running_binary().and_then(|current| connect::prefer_slugthug_binary(&current)),
    )?
    else {
        return Ok(());
    };
    if let Some(text) = try_step(
        "could not print the config snippet",
        connect::other_agent_instructions(&binary, install::slugaudit_dir().as_deref()),
    )? {
        cli::out(format_args!("{text}"))?;
    }
    Ok(())
}

fn confirm_serve() -> Result<bool, Box<dyn std::error::Error>> {
    cli::out(format_args!(
        "The MCP server (`serve`) is normally started automatically by your AI agent.\n\
         Start the server now? [y/N]: "
    ))?;
    Ok(crate::prompt::read_prompt_line()?.is_some_and(|line| crate::prompt::is_yes(&line)))
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
