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

#[test]
fn registry_entries_cite_a_reference_and_a_docs_page() {
    for a in AGENTS {
        assert!(
            !a.reference.trim().is_empty(),
            "{}: every entry must say what version its facts come from",
            a.id
        );
        assert!(
            a.docs_url.starts_with("https://"),
            "{}: docs link must be an https vendor page, got {:?}",
            a.id,
            a.docs_url
        );
        assert!(
            a.docs_url.len() > "https://".len() + 4,
            "{}: docs link must point at a page, not a bare scheme",
            a.id
        );
    }
    // A probed version and a documentation date must not be confused.
    for a in AGENTS
        .iter()
        .filter(|a| a.evidence != Evidence::VerifiedByProbe)
    {
        assert!(
            a.reference.contains("docs") || a.reference.contains("source"),
            "{}: non-probed entries must say the information is documentary, got {:?}",
            a.id,
            a.reference
        );
    }
}

fn refusal_text(agent: &crate::connect_agents::AgentDef) -> String {
    crate::connect_guidance::refusal(
        agent,
        Path::new("/bin/slugaudit"),
        Some(Path::new("/home/user")),
    )
    .to_string()
}

#[test]
fn every_refusal_carries_a_remedy_and_the_source_it_came_from() {
    for a in AGENTS {
        let text = refusal_text(a);
        assert!(
            text.contains("Reference: ") && text.contains(a.reference),
            "{}: refusal must cite its reference: {text}",
            a.id
        );
        assert!(
            text.contains(a.docs_url),
            "{}: refusal must link the vendor docs: {text}",
            a.id
        );
        assert!(
            text.ends_with(a.docs_url) || text.contains(&format!("Docs: {}", a.docs_url)),
            "{}: the docs link belongs on its own trailing line: {text}",
            a.id
        );
    }
    // The clients we refuse outright must also carry the paste-ready snippet.
    for a in AGENTS.iter().filter(|a| a.manual_steps) {
        let text = refusal_text(a);
        let remedy = a.guidance(Path::new("/bin/slugaudit")).expect("remedy");
        assert!(
            text.contains(&remedy),
            "{}: refusal must carry the paste-ready remedy: {text}",
            a.id
        );
    }
}

#[test]
fn success_and_failure_reports_always_name_their_source() {
    for a in AGENTS {
        let ok = crate::connect_guidance::success_lines(a, "Wrote mcpServers.slugaudit to /tmp/x");
        assert_eq!(ok[0], "Wrote mcpServers.slugaudit to /tmp/x");
        assert!(
            ok.iter()
                .any(|l| l == &format!("Reference: {}", a.reference)),
            "{}: a success must cite its reference: {ok:?}",
            a.id
        );
        let documented = a.evidence != Evidence::VerifiedByProbe;
        assert_eq!(
            ok.iter().any(|l| l.starts_with("Docs: ")),
            documented,
            "{}: the docs link is printed exactly when we acted on documentation",
            a.id
        );

        let failed = crate::connect_guidance::failure_lines(
            a,
            Path::new("/bin/slugaudit"),
            "claude mcp add slugaudit -- /bin/slugaudit",
            Some("claude mcp add slugaudit -- /bin/slugaudit"),
            "exit status 1",
        );
        assert!(
            failed
                .iter()
                .any(|l| l == &format!("Reference: {}", a.reference)),
            "{}: a failure must cite its reference: {failed:?}",
            a.id
        );
        assert!(
            failed.iter().any(|l| l.starts_with("  Do it by hand: ")),
            "{}: a failure must say how to finish by hand: {failed:?}",
            a.id
        );
        assert!(
            failed.iter().all(|l| !l.trim().is_empty()),
            "{}: no line of a failure report may be empty",
            a.id
        );
    }
}

#[test]
fn a_malformed_client_config_is_not_registered_and_says_why() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = directory.path().join("mcp.json");
    std::fs::write(&config, "{ not json").expect("write config");

    let (registered, logs) = crate::store::test_capture::capture_warns(|| {
        crate::connect_exec::has_slugaudit(&config, &["mcpServers"])
    });

    assert!(!registered);
    assert!(logs.contains("not valid JSON"), "{logs}");
}

#[test]
fn a_well_formed_client_config_with_the_entry_is_registered_silently() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = directory.path().join("mcp.json");
    std::fs::write(&config, r#"{"mcpServers":{"slugaudit":{}}}"#).expect("write config");

    let (registered, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-probe-has-slugaudit");
        crate::connect_exec::has_slugaudit(&config, &["mcpServers"])
    });

    assert!(registered);
    assert!(logs.contains("control-line-probe-has-slugaudit"));
    assert!(!logs.contains("not valid JSON"), "{logs}");
}

#[test]
fn non_json_client_configs_are_detected_without_a_format_warning() {
    for (id, body) in [
        (
            "codex",
            "[mcp_servers.slugaudit]\ncommand = \"/bin/slugaudit\"\n",
        ),
        (
            "grok",
            "[mcp_servers.slugaudit]\ncommand = \"/bin/slugaudit\"\n",
        ),
        (
            "hermes",
            "mcp_servers:\n  slugaudit:\n    command: /bin/slugaudit\n",
        ),
        (
            "goose",
            "extensions:\n  slugaudit:\n    cmd: /bin/slugaudit\n",
        ),
    ] {
        let home = tempfile::tempdir().expect("temp dir");
        let agent = find_agent(id).expect("agent is registered");
        let config = agent.config.resolve(home.path()).expect("config path");
        std::fs::create_dir_all(config.parent().expect("config parent")).expect("mkdir");
        std::fs::write(&config, body).expect("write config");

        let (connected, logs) = crate::store::test_capture::capture_warns(|| {
            tracing::warn!("control-line-probe-non-json");
            agent.config_has_slugaudit(home.path())
        });

        assert!(connected, "{id}: the entry must be detected");
        assert!(logs.contains("control-line-probe-non-json"), "{id}: {logs}");
        assert!(!logs.contains("not valid JSON"), "{id}: {logs}");
    }
}

#[test]
fn a_malformed_json_config_for_a_json_client_is_not_connected_and_warns() {
    let home = tempfile::tempdir().expect("temp dir");
    let cursor = find_agent("cursor").expect("cursor is registered");
    let config = cursor.config.resolve(home.path()).expect("config path");
    std::fs::create_dir_all(config.parent().expect("config parent")).expect("mkdir");
    std::fs::write(&config, "{ not json").expect("write config");

    let (connected, logs) =
        crate::store::test_capture::capture_warns(|| cursor.config_has_slugaudit(home.path()));

    assert!(!connected);
    assert!(logs.contains("not valid JSON"), "{logs}");
}

#[test]
fn an_unreadable_goose_config_is_not_connected_and_says_why() {
    let home = tempfile::tempdir().expect("temp dir");
    let goose = find_agent("goose").expect("goose is registered");
    let config = goose
        .config
        .resolve(home.path())
        .expect("goose config path");
    std::fs::create_dir_all(config.parent().expect("config parent")).expect("mkdir");
    let over_cap = usize::try_from(crate::util::MAX_FOREIGN_TEXT_BYTES).expect("cap fits") + 1;
    std::fs::write(&config, vec![b' '; over_cap]).expect("write oversized config");

    let (connected, logs) =
        crate::store::test_capture::capture_warns(|| goose.config_has_slugaudit(home.path()));

    assert!(!connected);
    assert!(logs.contains("could not read client config"), "{logs}");
}

#[test]
fn disconnect_fails_when_the_client_config_cannot_be_stat_ed() {
    let home = tempfile::tempdir().expect("temp dir");
    // `.cursor` is a file, so `.cursor/mcp.json` fails to stat with "not a
    // directory" instead of reporting "absent".
    std::fs::write(home.path().join(".cursor"), b"x").expect("write file");
    let cursor = find_agent("cursor").expect("cursor is registered");

    let result = crate::connect_exec::disconnect_agent(&cursor, Some(home.path()));

    let Err(crate::cli::ConnectError::Io(error)) = result else {
        panic!("expected Io, got {result:?}");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::NotADirectory,
        "the stat failure's cause must reach the caller: {error}"
    );
}

#[test]
fn the_installed_binary_check_fails_on_a_stat_error_instead_of_using_the_current_path() {
    let _guard = crate::util::TEST_ENV_LOCK.lock().expect("env lock");
    let temp = tempfile::tempdir().expect("temp dir");
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").expect("write file");

    let result = temp_env::with_var("SLUGTHUG_HOME", Some(file.as_os_str()), || {
        crate::connect::prefer_slugthug_binary(std::path::Path::new("/build/artifacts/slugaudit"))
    });

    assert!(result.is_err(), "{result:?}");
}
