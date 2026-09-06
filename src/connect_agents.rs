//! Supported AI agents and editors registry for SlugAudit connection.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Agy,
    Claude,
    Hermes,
    DashSeparator,
    Q,
    Crush,
    JsonFile,
    Zed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentDef {
    pub id: &'static str,
    pub display_name: &'static str,
    pub cli: Option<&'static str>,
    pub config_rel: Option<&'static str>,
    pub dialect: Dialect,
}

const fn ag(
    id: &'static str,
    name: &'static str,
    cli: Option<&'static str>,
    cfg: Option<&'static str>,
    d: Dialect,
) -> AgentDef {
    AgentDef {
        id,
        display_name: name,
        cli,
        config_rel: cfg,
        dialect: d,
    }
}

impl AgentDef {
    pub fn is_installed(&self, home: Option<&Path>) -> bool {
        if let Some(cli) = self.cli
            && which::which(cli).is_ok()
        {
            return true;
        }
        if let (Some(rel), Some(home)) = (self.config_rel, home) {
            let path = home.join(rel);
            if path.exists() {
                return true;
            }
            if let Some(parent) = path.parent()
                && parent.exists()
                && parent != home
            {
                return true;
            }
        }
        false
    }

    pub fn is_connected(&self, home: Option<&Path>) -> bool {
        match self.dialect {
            Dialect::JsonFile => {
                let Some(home) = home else { return false };
                let Some(rel) = self.config_rel else {
                    return false;
                };
                crate::connect_exec::json_contains_slugaudit(&home.join(rel))
            }
            Dialect::Zed => {
                let Some(home) = home else { return false };
                crate::connect_exec::zed_contains_slugaudit(&home.join(".config/zed/settings.json"))
            }
            _ => {
                if let (Some(rel), Some(home)) = (self.config_rel, home) {
                    let path = home.join(rel);
                    if path.is_file()
                        && let Ok(c) = std::fs::read_to_string(&path)
                        && c.contains("slugaudit")
                    {
                        return true;
                    }
                }
                if let Some(cli) = self.cli
                    && which::which(cli).is_ok()
                {
                    return crate::connect_exec::cli_check_connected(cli);
                }
                false
            }
        }
    }
}

#[rustfmt::skip]
pub const AGENTS: &[AgentDef] = &[
    ag("agy", "Antigravity (agy)", Some("agy"), None, Dialect::Agy),
    ag("gemini", "Gemini CLI", Some("gemini"), None, Dialect::Agy),
    ag("claude", "Claude Code", Some("claude"), Some(".claude.json"), Dialect::Claude),
    ag("hermes", "Hermes Agent", Some("hermes"), Some(".hermes/config.yaml"), Dialect::Hermes),
    ag("copilot", "GitHub Copilot", Some("copilot"), Some(".copilot/mcp-config.json"), Dialect::DashSeparator),
    ag("codex", "Codex CLI", Some("codex"), Some(".codex/config.toml"), Dialect::DashSeparator),
    ag("cursor", "Cursor IDE", None, Some(".cursor/mcp.json"), Dialect::JsonFile),
    ag("windsurf", "Windsurf", None, Some(".codeium/windsurf/mcp_config.json"), Dialect::JsonFile),
    ag("trae", "Trae AI IDE", None, Some(".trae/mcp.json"), Dialect::JsonFile),
    ag("opencode", "OpenCode", Some("opencode"), Some(".config/opencode/opencode.json"), Dialect::JsonFile),
    ag("pi", "Pi Agent", None, Some(".pi/agent/mcp.json"), Dialect::JsonFile),
    ag("omp", "Oh My Pi (omp)", None, Some(".omp/mcp.json"), Dialect::JsonFile),
    ag("amp", "Amp Agent", Some("amp"), None, Dialect::DashSeparator),
    ag("bob", "Bob Agent", Some("bob"), Some(".bob/mcp_settings.json"), Dialect::DashSeparator),
    ag("grok", "Grok CLI", Some("grok"), Some(".grok/config.toml"), Dialect::DashSeparator),
    ag("goose", "Goose CLI", Some("goose"), Some(".config/goose/config.yaml"), Dialect::Agy),
    ag("1mcp", "1MCP Agent", Some("1mcp"), None, Dialect::DashSeparator),
    ag("openhands", "OpenHands", Some("openhands"), None, Dialect::Agy),
    ag("kimi", "Kimi Code", Some("kimi"), Some(".kimi-code/mcp.json"), Dialect::Agy),
    ag("droid", "Droid (Factory)", Some("droid"), None, Dialect::Agy),
    ag("crush", "Crush", Some("crush"), Some(".config/crush/crush.json"), Dialect::Crush),
    ag("q", "Amazon Q Developer", Some("q"), None, Dialect::Q),
    ag("pearai", "PearAI", None, Some(".pearai/mcp.json"), Dialect::JsonFile),
    ag("warp", "Warp Terminal", None, Some(".warp/mcp.json"), Dialect::JsonFile),
    ag("claude-desktop", "Claude Desktop", None, Some(".config/Claude/claude_desktop_config.json"), Dialect::JsonFile),
    ag("zed", "Zed Editor", None, Some(".config/zed/settings.json"), Dialect::Zed),
];

pub fn find_agent(name: &str) -> Option<AgentDef> {
    let n = name.trim().to_ascii_lowercase();
    let n = match n.as_str() {
        "claude-code" | "claude_code" => "claude",
        "antigravity" => "agy",
        "github-copilot" | "github_copilot" => "copilot",
        "oh-my-pi" | "oh_my_pi" => "omp",
        other => other,
    };
    AGENTS
        .iter()
        .copied()
        .find(|a| a.id == n || a.display_name.eq_ignore_ascii_case(n))
}

pub fn detected_agents(home: Option<&Path>) -> Vec<AgentDef> {
    AGENTS
        .iter()
        .copied()
        .filter(|a| a.is_installed(home))
        .collect()
}
