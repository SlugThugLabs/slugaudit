//! Refusal, report and config-detection invariants for the agent registry.
//!
//! Split from `connect_registry_tests.rs`, which keeps the table-shape
//! invariants; wired from `connect_agents.rs` the same way.

use super::*;

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
