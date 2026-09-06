//! ANSI rendering and formatting helpers for the interactive setup menu.
#![allow(clippy::print_stdout)]

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

pub(crate) fn render_menu(style: &Style) -> String {
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

pub(crate) fn section_label(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("── ")?;
    let (label, _after) = rest.split_once(" ─")?;
    if label.is_empty() || !label.chars().all(|c| c.is_alphanumeric() || c == ' ') {
        return None;
    }
    Some(label)
}

pub(crate) fn other_agent_instructions(binary: &Path) -> String {
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
