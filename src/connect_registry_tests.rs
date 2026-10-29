//! Table-driven invariants for the agent registry.
//!
//! Wired from `connect_agents.rs` so the table and its invariants live in the
//! same slice; the behaviour tests (fake CLIs, config writes) stay in
//! `connect_tests.rs`.

use super::*;

#[test]
fn find_agent_finds_standard_and_aliased_names() {
    assert_eq!(find_agent("agy").map(|a| a.id), Some("agy"));
    assert_eq!(find_agent("antigravity").map(|a| a.id), Some("agy"));
    assert_eq!(find_agent("claude").map(|a| a.id), Some("claude"));
    assert_eq!(find_agent("claude-code").map(|a| a.id), Some("claude"));
    assert_eq!(find_agent("claude_code").map(|a| a.id), Some("claude"));
    assert_eq!(find_agent("cursor").map(|a| a.id), Some("cursor"));
    assert_eq!(find_agent("zed").map(|a| a.id), Some("zed"));
    assert_eq!(find_agent("nonexistent_agent"), None);
}

#[test]
fn registry_entries_declare_an_executable_or_a_config_file() {
    assert_eq!(AGENTS.len(), 23, "the reconciled registry holds 23 clients");
    for a in AGENTS {
        match a.strategy {
            Strategy::CliDirect => assert!(
                a.cli.is_some() || a.config.is_usable(),
                "{}: CliDirect needs an executable or a config file",
                a.id
            ),
            Strategy::CliInteractiveOnly => {
                assert!(
                    a.cli.is_none(),
                    "{}: an interactive-only client must not invoke a CLI",
                    a.id
                );
                assert!(
                    a.config.is_usable(),
                    "{}: an interactive-only client needs a config file",
                    a.id
                );
            }
            Strategy::ConfigFile => {
                assert!(
                    a.config.is_usable(),
                    "{}: a config-file entry needs a verified path",
                    a.id
                );
            }
            Strategy::Unknown => panic!(
                "{}: no client may be left without an established mechanism",
                a.id
            ),
        }
    }
}

#[test]
fn registry_entries_carry_an_evidence_status() {
    for a in AGENTS {
        assert!(
            !a.note.is_empty(),
            "{}: every entry must state what is verified or open",
            a.id
        );
    }
    // Probes against the real binaries override both doc passes; these are the
    // clients the passes disagreed about or that could be settled on the box.
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
        assert_eq!(
            find_agent(id).expect("agent").evidence,
            Evidence::VerifiedByProbe,
            "{id} was probed"
        );
    }
    // Documented-only entries: GUI apps with no CLI to run, so their config
    // path comes from vendor documentation alone.
    for id in ["windsurf", "warp", "claude-desktop"] {
        assert_eq!(
            find_agent(id).expect("agent").evidence,
            Evidence::DocVerified,
            "{id} is doc-verified"
        );
    }
    // Source-verified entries: the client's own code names the path and shape.
    for id in ["zed", "omp"] {
        assert_eq!(
            find_agent(id).expect("agent").evidence,
            Evidence::VerifiedFromSource,
            "{id} is source-verified"
        );
    }
    // The Unknown state stays part of the model, but every client now has an
    // established mechanism, so no live entry needs it.
    assert!(
        AGENTS.iter().all(|a| a.strategy != Strategy::Unknown),
        "no client is left without a known strategy"
    );
}

#[test]
fn registry_never_uses_a_display_name_as_the_executable() {
    for a in AGENTS {
        for cli in a.cli.into_iter().chain(a.verify_cli) {
            assert!(!cli.is_empty(), "{}: empty executable", a.id);
            assert!(
                !cli.contains(char::is_whitespace),
                "{}: executable {cli:?} must be a single token",
                a.id
            );
            assert!(
                !cli.contains('(') && !cli.contains(')'),
                "{}: executable {cli:?} must be a bare command, not a label",
                a.id
            );
            assert_eq!(
                cli,
                cli.to_ascii_lowercase(),
                "{}: executable {cli:?} must be a plain command name",
                a.id
            );
            assert_ne!(
                cli, a.display_name,
                "{}: the display name is not an executable",
                a.id
            );
        }
    }
    // A client's executable is its own id except where the vendor ships a
    // differently named binary; the exceptions are explicit, not accidental.
    let renamed: Vec<_> = AGENTS
        .iter()
        .filter_map(|a| a.cli.filter(|cli| *cli != a.id).map(|cli| (a.id, cli)))
        .collect();
    assert_eq!(
        renamed,
        [("warp", "oz")],
        "only warp ships a differently named CLI"
    );
    let verifiers: Vec<_> = AGENTS
        .iter()
        .filter_map(|a| a.verify_cli.map(|cli| (a.id, cli)))
        .collect();
    assert_eq!(
        verifiers,
        [("cursor", "agent")],
        "cursor's agent binary can verify but not register"
    );
}

#[test]
fn registry_file_fallbacks_keep_a_real_executable_and_path() {
    let fallbacks: Vec<_> = AGENTS
        .iter()
        .filter(|a| a.file_fallback)
        .map(|a| a.id)
        .collect();
    assert_eq!(fallbacks, ["opencode", "bob", "pi"]);
    for a in AGENTS.iter().filter(|a| a.file_fallback) {
        assert!(
            a.cli.is_some(),
            "{}: a file fallback still needs a CLI to try first",
            a.id
        );
        assert!(
            a.config.is_usable(),
            "{}: a file fallback needs a config path",
            a.id
        );
        assert!(
            a.writes_config_file(),
            "{}: a fallback must be allowed to write the config file",
            a.id
        );
    }
}

#[test]
fn registry_cli_argument_grammar_is_preserved_per_client() {
    assert_eq!(
        find_agent("claude").expect("claude").dialect,
        Dialect::Claude
    );
    assert_eq!(
        find_agent("gemini").expect("gemini").dialect,
        Dialect::GeminiScope,
        "gemini's --scope defaults to project, so a global registration must pass -s user"
    );
    assert_eq!(
        find_agent("hermes").expect("hermes").dialect,
        Dialect::Hermes
    );
    assert_eq!(find_agent("crush").expect("crush").dialect, Dialect::Crush);
    assert_eq!(find_agent("q").expect("q").dialect, Dialect::Q);
    assert_eq!(
        find_agent("pi").expect("pi").dialect,
        Dialect::DashSeparator
    );
    assert_eq!(
        find_agent("opencode").expect("opencode").dialect,
        Dialect::DashSeparator,
        "opencode takes the command after `--`"
    );
    assert_eq!(
        find_agent("copilot").expect("copilot").dialect,
        Dialect::DashSeparator,
        "copilot accepts neither a scope flag nor --command"
    );
    assert_eq!(
        find_agent("amp").expect("amp").dialect,
        Dialect::DashSeparator
    );
    assert_eq!(
        find_agent("openhands").expect("openhands").dialect,
        Dialect::TransportStdio,
        "openhands requires an explicit stdio transport"
    );
    assert_eq!(
        find_agent("bob").expect("bob").dialect,
        Dialect::BobTransportScope,
        "bob spells the flag --transport and defaults to workspace scope"
    );
    assert_eq!(
        find_agent("droid").expect("droid").dialect,
        Dialect::DroidType,
        "droid spells the flag --type, not --transport"
    );
}

#[test]
fn registry_shape_and_path_models_match_the_documented_forms() {
    let zed = find_agent("zed").expect("zed");
    assert_eq!(zed.shape, ConfigShape::ZedContextServer);
    let zed_entry = zed
        .shape
        .entry(Path::new("/bin/slugaudit"))
        .expect("UTF-8 binary path");
    assert_eq!(
        zed_entry["command"], "/bin/slugaudit",
        "zed's command is a string, not a nested object"
    );
    assert!(
        zed_entry["args"].is_array(),
        "zed keeps a sibling args array"
    );
    assert_eq!(zed.shape.write_section(), Some("context_servers"));

    assert_eq!(
        find_agent("opencode").expect("opencode").shape,
        ConfigShape::OpenCodeLocal
    );
    assert_eq!(
        find_agent("opencode").expect("opencode").config,
        ConfigPath::Home(".config/opencode/opencode.jsonc"),
        "the probe wrote the .jsonc variant"
    );
    let opencode_entry = ConfigShape::OpenCodeLocal
        .entry(Path::new("/bin/slugaudit"))
        .expect("UTF-8 binary path");
    assert_eq!(opencode_entry["type"], "local");
    assert_eq!(opencode_entry["command"][0], "/bin/slugaudit");
    assert_eq!(opencode_entry["enabled"], true);

    assert_eq!(
        find_agent("goose").expect("goose").shape,
        ConfigShape::GooseExtensions
    );

    let desktop = find_agent("claude-desktop").expect("claude-desktop");
    let home = Path::new("/home/user");
    let resolved = desktop.config.resolve(home).expect("per-os config path");
    assert!(resolved.ends_with("claude_desktop_config.json"));
    #[cfg(target_os = "linux")]
    assert_eq!(
        resolved,
        home.join(".config/Claude/claude_desktop_config.json")
    );

    let manual: Vec<_> = AGENTS
        .iter()
        .filter(|a| a.manual_steps)
        .map(|a| a.id)
        .collect();
    assert_eq!(
        manual,
        ["goose", "crush"],
        "these clients cannot be registered by an automated write"
    );
    for a in AGENTS.iter().filter(|a| a.manual_steps) {
        assert!(
            !a.remedy.trim().is_empty(),
            "{}: a refusal needs a paste-ready remedy",
            a.id
        );
        assert!(
            a.blocker().is_some(),
            "{}: a manual entry must block the automated path",
            a.id
        );
    }
    for a in AGENTS.iter().filter(|a| !a.manual_steps) {
        assert!(
            a.remedy.is_empty(),
            "{}: a remedy belongs only to a manual entry",
            a.id
        );
        assert!(
            a.blocker().is_none(),
            "{}: only a manual entry may block the automated path",
            a.id
        );
    }
}
