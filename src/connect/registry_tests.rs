//! Table-driven invariants for the agent registry.
//!
//! Wired from `agents.rs` so the table and its invariants live in the same
//! slice; the behaviour tests (fake CLIs, config writes) stay in `mod_tests.rs`.

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq, ensure_ne};
use std::path::Path;

#[test]
fn find_agent_finds_standard_and_aliased_names() -> TestResult {
    ensure_eq(
        &find_agent("agy").map(|a| a.id),
        &Some("agy"),
        "resolves agy id",
    )?;
    ensure_eq(
        &find_agent("antigravity").map(|a| a.id),
        &Some("agy"),
        "resolves antigravity alias to agy",
    )?;
    ensure_eq(
        &find_agent("claude").map(|a| a.id),
        &Some("claude"),
        "resolves claude id",
    )?;
    ensure_eq(
        &find_agent("claude-code").map(|a| a.id),
        &Some("claude"),
        "resolves claude-code alias to claude",
    )?;
    ensure_eq(
        &find_agent("claude_code").map(|a| a.id),
        &Some("claude"),
        "resolves claude_code alias to claude",
    )?;
    ensure_eq(
        &find_agent("cursor").map(|a| a.id),
        &Some("cursor"),
        "resolves cursor id",
    )?;
    ensure_eq(
        &find_agent("zed").map(|a| a.id),
        &Some("zed"),
        "resolves zed id",
    )?;
    ensure_eq(
        &find_agent("nonexistent_agent"),
        &None,
        "unknown agent returns None",
    )?;
    Ok(())
}

#[test]
fn registry_entries_declare_an_executable_or_a_config_file() -> TestResult {
    ensure_eq(
        &AGENTS.len(),
        &23,
        "the reconciled registry holds 23 clients",
    )?;
    for a in AGENTS {
        match a.strategy {
            Strategy::CliDirect => ensure(
                a.cli.is_some() || a.config.is_usable(),
                format!("{}: CliDirect needs an executable or a config file", a.id),
            )?,
            Strategy::CliInteractiveOnly => {
                ensure(
                    a.cli.is_none(),
                    format!("{}: an interactive-only client must not invoke a CLI", a.id),
                )?;
                ensure(
                    a.config.is_usable(),
                    format!("{}: an interactive-only client needs a config file", a.id),
                )?;
            }
            Strategy::ConfigFile => {
                ensure(
                    a.config.is_usable(),
                    format!("{}: a config-file entry needs a verified path", a.id),
                )?;
            }
            Strategy::Unknown => {
                return Err(format!(
                    "{}: no client may be left without an established mechanism",
                    a.id
                ))
                .check("unexpected outcome");
            }
        }
    }
    Ok(())
}

#[test]
fn registry_entries_carry_an_evidence_status() -> TestResult {
    for a in AGENTS {
        ensure(
            !a.note.is_empty(),
            format!("{}: every entry must state what is verified or open", a.id),
        )?;
    }
    for id in [
        "agy",
        "claude",
        "hermes",
        "codex",
        "copilot",
        "opencode",
        "amp",
        "bob",
        "grok",
        "goose",
        "gemini",
        "openhands",
        "kimi",
        "droid",
        "crush",
        "q",
        "cursor",
        "pi",
    ] {
        ensure_eq(
            &find_agent(id).check("agent")?.evidence,
            &Evidence::VerifiedByProbe,
            format!("{id} was probed"),
        )?;
    }
    for id in ["windsurf", "warp", "claude-desktop"] {
        ensure_eq(
            &find_agent(id).check("agent")?.evidence,
            &Evidence::DocVerified,
            format!("{id} is doc-verified"),
        )?;
    }
    for id in ["zed", "omp"] {
        ensure_eq(
            &find_agent(id).check("agent")?.evidence,
            &Evidence::VerifiedFromSource,
            format!("{id} is source-verified"),
        )?;
    }
    ensure(
        AGENTS.iter().all(|a| a.strategy != Strategy::Unknown),
        "no client is left without a known strategy",
    )?;
    Ok(())
}

#[test]
fn registry_never_uses_a_display_name_as_the_executable() -> TestResult {
    for a in AGENTS {
        for cli in a.cli.into_iter().chain(a.verify_cli) {
            ensure(!cli.is_empty(), format!("{}: empty executable", a.id))?;
            ensure(
                !cli.contains(char::is_whitespace),
                format!("{}: executable {cli:?} must be a single token", a.id),
            )?;
            ensure(
                !cli.contains('(') && !cli.contains(')'),
                format!(
                    "{}: executable {cli:?} must be a bare command, not a label",
                    a.id
                ),
            )?;
            ensure_eq(
                &cli,
                &cli.to_ascii_lowercase(),
                format!("{}: executable {cli:?} must be a plain command name", a.id),
            )?;
            ensure_ne(
                &cli,
                &a.display_name,
                format!("{}: the display name is not an executable", a.id),
            )?;
        }
    }
    let renamed: Vec<_> = AGENTS
        .iter()
        .filter_map(|a| a.cli.filter(|cli| *cli != a.id).map(|cli| (a.id, cli)))
        .collect();
    ensure_eq(
        &renamed,
        &[("warp", "oz")],
        "only warp ships a differently named CLI",
    )?;
    let verifiers: Vec<_> = AGENTS
        .iter()
        .filter_map(|a| a.verify_cli.map(|cli| (a.id, cli)))
        .collect();
    ensure_eq(
        &verifiers,
        &[("cursor", "agent")],
        "cursor's agent binary can verify but not register",
    )?;
    Ok(())
}

#[test]
fn registry_file_fallbacks_keep_a_real_executable_and_path() -> TestResult {
    let fallbacks: Vec<_> = AGENTS
        .iter()
        .filter(|a| a.file_fallback)
        .map(|a| a.id)
        .collect();
    ensure_eq(
        &fallbacks,
        &["opencode", "bob", "pi"],
        "opencode, bob, and pi carry file fallbacks",
    )?;
    for a in AGENTS.iter().filter(|a| a.file_fallback) {
        a.cli.as_ref().check("fallback entry has a CLI")?;
        ensure(
            a.config.is_usable(),
            format!("{}: a file fallback needs a config path", a.id),
        )?;
        ensure(
            a.writes_config_file(),
            format!(
                "{}: a fallback must be allowed to write the config file",
                a.id
            ),
        )?;
    }
    Ok(())
}

#[test]
fn registry_cli_argument_grammar_is_preserved_per_client() -> TestResult {
    ensure_eq(
        &find_agent("claude").check("claude")?.dialect,
        &Dialect::Claude,
        "claude uses Claude dialect",
    )?;
    ensure_eq(
        &find_agent("gemini").check("gemini")?.dialect,
        &Dialect::GeminiScope,
        "gemini's --scope defaults to project, so a global registration must pass -s user",
    )?;
    ensure_eq(
        &find_agent("hermes").check("hermes")?.dialect,
        &Dialect::Hermes,
        "hermes uses Hermes dialect",
    )?;
    ensure_eq(
        &find_agent("crush").check("crush")?.dialect,
        &Dialect::Crush,
        "crush uses Crush dialect",
    )?;
    ensure_eq(
        &find_agent("q").check("q")?.dialect,
        &Dialect::Q,
        "amazon q uses Q dialect",
    )?;
    ensure_eq(
        &find_agent("grok").check("grok")?.dialect,
        &Dialect::DashSeparatorUserScope,
        "grok passes explicit --scope user before --",
    )?;
    ensure_eq(
        &find_agent("pi").check("pi")?.dialect,
        &Dialect::DashSeparator,
        "pi uses DashSeparator dialect",
    )?;
    ensure_eq(
        &find_agent("opencode").check("opencode")?.dialect,
        &Dialect::DashSeparator,
        "opencode takes the command after `--`",
    )?;
    ensure_eq(
        &find_agent("copilot").check("copilot")?.dialect,
        &Dialect::DashSeparator,
        "copilot accepts neither a scope flag nor --command",
    )?;
    ensure_eq(
        &find_agent("amp").check("amp")?.dialect,
        &Dialect::DashSeparator,
        "amp uses DashSeparator dialect",
    )?;
    ensure_eq(
        &find_agent("openhands").check("openhands")?.dialect,
        &Dialect::TransportStdio,
        "openhands requires an explicit stdio transport",
    )?;
    ensure_eq(
        &find_agent("bob").check("bob")?.dialect,
        &Dialect::BobTransportScope,
        "bob spells the flag --transport and defaults to workspace scope",
    )?;
    ensure_eq(
        &find_agent("droid").check("droid")?.dialect,
        &Dialect::DroidType,
        "droid spells the flag --type, not --transport",
    )?;
    Ok(())
}

/// Only the clients that need manual steps block the automated path, and each
/// of them carries a paste-ready remedy.
fn check_manual_agents_carry_a_remedy_and_only_they_block() -> TestResult {
    let manual: Vec<_> = AGENTS
        .iter()
        .filter(|a| a.manual_steps)
        .map(|a| a.id)
        .collect();
    ensure_eq(
        &manual,
        &["goose", "crush"],
        "these clients cannot be registered by an automated write",
    )?;
    for a in AGENTS.iter().filter(|a| a.manual_steps) {
        ensure(
            !a.remedy.trim().is_empty(),
            format!("{}: a refusal needs a paste-ready remedy", a.id),
        )?;
        a.blocker().check("manual agent exposes a blocker")?;
    }
    for a in AGENTS.iter().filter(|a| !a.manual_steps) {
        ensure(
            a.remedy.is_empty(),
            format!("{}: a remedy belongs only to a manual entry", a.id),
        )?;
        ensure(
            a.blocker().is_none(),
            format!("{}: only a manual entry may block the automated path", a.id),
        )?;
    }
    Ok(())
}

#[test]
fn registry_shape_and_path_models_match_the_documented_forms() -> TestResult {
    let zed = find_agent("zed").check("zed")?;
    ensure_eq(
        &zed.shape,
        &ConfigShape::ZedContextServer,
        "zed uses ZedContextServer shape",
    )?;
    let zed_entry = zed
        .shape
        .entry(Path::new("/bin/slugaudit"))
        .check("UTF-8 binary path")?;
    ensure_eq(
        zed_entry.get("command").check("zed entry has a command")?,
        &"/bin/slugaudit",
        "zed's command is a string, not a nested object",
    )?;
    ensure(
        zed_entry
            .get("args")
            .is_some_and(serde_json::Value::is_array),
        "zed keeps a sibling args array",
    )?;
    ensure_eq(
        &zed.shape.write_section(),
        &Some("context_servers"),
        "zed writes context_servers section",
    )?;

    ensure_eq(
        &find_agent("opencode").check("opencode")?.shape,
        &ConfigShape::OpenCodeLocal,
        "opencode uses OpenCodeLocal shape",
    )?;
    ensure_eq(
        &find_agent("opencode").check("opencode")?.config,
        &ConfigPath::Home(".config/opencode/opencode.jsonc"),
        "the probe wrote the .jsonc variant",
    )?;
    let opencode_entry = ConfigShape::OpenCodeLocal
        .entry(Path::new("/bin/slugaudit"))
        .check("UTF-8 binary path")?;
    ensure_eq(
        opencode_entry
            .get("type")
            .check("opencode entry has a type")?,
        &"local",
        "opencode entry type is local",
    )?;
    ensure_eq(
        opencode_entry
            .get("command")
            .and_then(|command| command.get(0))
            .check("opencode entry has a command array")?,
        &"/bin/slugaudit",
        "opencode command array starts with binary path",
    )?;
    ensure_eq(
        opencode_entry
            .get("enabled")
            .check("opencode entry has an enabled flag")?,
        &true,
        "opencode entry sets enabled to true",
    )?;

    ensure_eq(
        &find_agent("goose").check("goose")?.shape,
        &ConfigShape::GooseExtensions,
        "goose uses GooseExtensions shape",
    )?;

    let desktop = find_agent("claude-desktop").check("claude-desktop")?;
    let home = Path::new("/home/user");
    let resolved = desktop.config.resolve(home).check("per-os config path")?;
    ensure(
        resolved.ends_with("claude_desktop_config.json"),
        "claude-desktop resolves to claude_desktop_config.json",
    )?;
    #[cfg(target_os = "linux")]
    ensure_eq(
        &resolved,
        &home.join(".config/Claude/claude_desktop_config.json"),
        "claude-desktop Linux path resolves under .config/Claude",
    )?;

    check_manual_agents_carry_a_remedy_and_only_they_block()?;
    Ok(())
}
