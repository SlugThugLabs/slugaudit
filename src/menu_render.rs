//! ANSI rendering and formatting helpers for the interactive setup menu.

use crate::cli::ConnectError;
use crate::connect::registry_types::binary_text;
use crate::util::Style;
use std::path::Path;

pub(crate) const MENU: &str = "  ____  _             _             _ _ _   
 / ___|| |_   _  __ _/ \\  _   _  __| (_) |_ 
 \\___ \\| | | | |/ _` / _ \\| | | |/ _` | | __|
  ___) | | |_| | (_| / ___ \\ |_| | (_| | | |_ 
 |____/|_|\\__,_|\\__, /_/   \\_\\__,_|\\__,_|_|\\__|
                |___/                          
┌─────────────────────────────────────────────┐
│            SlugAudit setup                  │
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

Choose an option [1-6]: ";

pub(crate) fn render_menu(style: &Style) -> String {
    if !style.enabled() {
        return MENU.to_owned();
    }
    let mut out = String::with_capacity(MENU.len() + 96);
    let line_count = MENU.lines().count();
    for (index, line) in MENU.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(label) = section_label(trimmed) {
            // `label` is a substring of `line`, so both splits succeed; a line
            // that somehow fails to split is emitted unstyled.
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
        // The last line is the prompt: the cursor stays on it, as in the plain menu.
        if index + 1 < line_count {
            out.push('\n');
        }
    }
    out
}

pub(crate) fn section_label(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("── ")?;
    let (label, _after) = rest.split_once(" ─")?;
    if label.is_empty() || !label.chars().all(|c| c.is_alphanumeric() || c == ' ') {
        return None;
    }
    Some(label)
}

/// The instructions and JSON config snippet for an MCP client that `connect`
/// does not know. `install_dir` is the SlugAudit install directory, when one
/// can be resolved; the stable command path is printed under it.
///
/// # Errors
///
/// A binary path that is not valid UTF-8 is refused, since it cannot be
/// written into a client's JSON config; a JSON encoding failure is returned.
pub(crate) fn other_agent_instructions(
    binary: &Path,
    install_dir: Option<&Path>,
) -> Result<String, ConnectError> {
    let command = binary_text(binary)?;
    let snippet = serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { "slugaudit": { "command": command, "args": [] } }
    }))?;
    let stable = match install_dir {
        Some(dir) => format!("{}\n", dir.join("slugaudit").display()),
        None => "(unavailable: neither SLUGTHUG_HOME nor HOME is set)\n".to_owned(),
    };
    Ok(format!(
        concat!(
            "SlugAudit is a standard stdio MCP server, so any MCP client can add it:\n\n",
            "  Server name:  slugaudit\n",
            "  Command:      {command}\n",
            "  Arguments:    (none - `serve` is the default)\n\n",
            "Config entry for `.mcp.json` or equivalent:\n\n",
            "{snippet}\n\n",
            "Stable command path after install: {stable}",
        ),
        command = command,
        snippet = snippet,
        stable = stable,
    ))
}
