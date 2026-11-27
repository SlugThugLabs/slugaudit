//! Refusal, report and config-detection invariants for the agent registry.
//!
//! Kept apart from `registry_tests.rs`, which holds the table-shape
//! invariants; wired from `agents.rs` the same way.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::path::Path;

#[test]
fn registry_entries_cite_a_reference_and_a_docs_page() -> TestResult {
    for a in AGENTS {
        ensure(
            !a.reference.trim().is_empty(),
            format!(
                "{}: every entry must say what version its facts come from",
                a.id
            ),
        )?;
        ensure(
            a.docs_url.starts_with("https://"),
            format!(
                "{}: docs link must be an https vendor page, got {:?}",
                a.id, a.docs_url
            ),
        )?;
        ensure(
            a.docs_url.len() > 12,
            format!(
                "{}: docs link must point at a page, not a bare scheme",
                a.id
            ),
        )?;
    }
    for a in AGENTS
        .iter()
        .filter(|a| a.evidence != Evidence::VerifiedByProbe)
    {
        ensure(
            a.reference.contains("docs") || a.reference.contains("source"),
            format!(
                "{}: non-probed entries must say the information is documentary, got {:?}",
                a.id, a.reference
            ),
        )?;
    }
    Ok(())
}

fn refusal_text(agent: &AgentDef) -> String {
    super::super::guidance::refusal(
        agent,
        Path::new("/bin/slugaudit"),
        Some(Path::new("/home/user")),
    )
    .to_string()
}

#[test]
fn every_refusal_carries_a_remedy_and_the_source_it_came_from() -> TestResult {
    for a in AGENTS {
        let text = refusal_text(a);
        ensure(
            text.contains("Reference: ") && text.contains(a.reference),
            format!("{}: refusal must cite its reference: {text}", a.id),
        )?;
        ensure(
            text.contains(a.docs_url),
            format!("{}: refusal must link the vendor docs: {text}", a.id),
        )?;
        ensure(
            text.ends_with(a.docs_url) || text.contains(&format!("Docs: {}", a.docs_url)),
            format!(
                "{}: the docs link belongs on its own trailing line: {text}",
                a.id
            ),
        )?;
    }
    for a in AGENTS.iter().filter(|a| a.manual_steps) {
        let text = refusal_text(a);
        let remedy = a.guidance(Path::new("/bin/slugaudit")).check("remedy")?;
        ensure(
            text.contains(&remedy),
            format!(
                "{}: refusal must carry the paste-ready remedy: {text}",
                a.id
            ),
        )?;
    }
    Ok(())
}

#[test]
fn success_and_failure_reports_always_name_their_source() -> TestResult {
    for a in AGENTS {
        let ok = super::super::guidance::success_lines(a, "Wrote mcpServers.slugaudit to /tmp/x");
        ensure_eq(
            ok.first()
                .check("the success lines start with the wrote line")?,
            &"Wrote mcpServers.slugaudit to /tmp/x",
            "first success line states action performed",
        )?;
        ensure(
            ok.iter()
                .any(|l| l == &format!("Reference: {}", a.reference)),
            format!("{}: a success must cite its reference: {ok:?}", a.id),
        )?;
        let documented = a.evidence != Evidence::VerifiedByProbe;
        ensure_eq(
            &ok.iter().any(|l| l.starts_with("Docs: ")),
            &documented,
            format!(
                "{}: the docs link is printed exactly when we acted on documentation",
                a.id
            ),
        )?;

        let failed = super::super::guidance::failure_lines(
            a,
            Path::new("/bin/slugaudit"),
            "claude mcp add slugaudit -- /bin/slugaudit",
            Some("claude mcp add slugaudit -- /bin/slugaudit"),
            "exit status 1",
        );
        ensure(
            failed
                .iter()
                .any(|l| l == &format!("Reference: {}", a.reference)),
            format!("{}: a failure must cite its reference: {failed:?}", a.id),
        )?;
        ensure(
            failed.iter().any(|l| l.starts_with("  Do it by hand: ")),
            format!(
                "{}: a failure must say how to finish by hand: {failed:?}",
                a.id
            ),
        )?;
        ensure(
            failed.iter().all(|l| !l.trim().is_empty()),
            format!("{}: no line of a failure report may be empty", a.id),
        )?;
    }
    Ok(())
}

#[test]
fn a_malformed_client_config_is_an_error_that_keeps_the_json_cause() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let config = directory.path().join("mcp.json");
    std::fs::write(&config, "{ not json").check("write config")?;

    let error = crate::connect::has_slugaudit(&config, &["mcpServers"])
        .check_err("a malformed config cannot be judged")?;

    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::InvalidData,
        "malformed JSON yields InvalidData kind",
    )?;
    ensure(
        error
            .get_ref()
            .is_some_and(<dyn std::error::Error + Send + Sync + 'static>::is::<serde_json::Error>),
        format!("the JSON error is kept as the source: {error:?}"),
    )?;
    Ok(())
}

#[test]
fn a_well_formed_client_config_with_the_entry_is_registered_silently() -> TestResult {
    let directory = tempfile::tempdir().check("temp dir")?;
    let config = directory.path().join("mcp.json");
    std::fs::write(&config, r#"{"mcpServers":{"slugaudit":{}}}"#).check("write config")?;

    let (registered, logs) = crate::store::test_capture::capture_warns(|| {
        tracing::warn!("control-line-probe-has-slugaudit");
        crate::connect::has_slugaudit(&config, &["mcpServers"])
    })?;

    ensure(
        registered.check("probe config")?,
        "well-formed config with slugaudit key reports registered",
    )?;
    ensure(
        logs.contains("control-line-probe-has-slugaudit"),
        "log capture recorded control warning",
    )?;
    ensure(!logs.contains("not valid JSON"), &logs)?;
    Ok(())
}

#[test]
fn non_json_client_configs_are_detected_without_a_format_warning() -> TestResult {
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
        let home = tempfile::tempdir().check("temp dir")?;
        let agent = find_agent(id).check("agent is registered")?;
        let config = agent.config.resolve(home.path()).check("config path")?;
        std::fs::create_dir_all(config.parent().check("config parent")?).check("mkdir")?;
        std::fs::write(&config, body).check("write config")?;

        let (connected, logs) = crate::store::test_capture::capture_warns(|| {
            tracing::warn!("control-line-probe-non-json");
            agent.config_has_slugaudit(home.path())
        })?;

        ensure(
            connected.check("probe config")?,
            format!("{id}: the entry must be detected"),
        )?;
        ensure(
            logs.contains("control-line-probe-non-json"),
            format!("{id}: {logs}"),
        )?;
        ensure(!logs.contains("not valid JSON"), format!("{id}: {logs}"))?;
    }
    Ok(())
}

#[test]
fn hash_comments_mentioning_slugaudit_do_not_false_positive_in_non_json_configs() -> TestResult {
    for (id, body) in [
        (
            "codex",
            "# [mcp_servers.slugaudit]\n[mcp_servers.other]\ncommand = \"/bin/other\" # not slugaudit\n",
        ),
        (
            "grok",
            "  # disabled slugaudit\n[mcp_servers.other]\ncommand = \"/bin/other\"#slugaudit\n",
        ),
        (
            "hermes",
            "# slugaudit removed\nmcp_servers:\n  other:\n    command: /bin/other # slugaudit\n",
        ),
        (
            "goose",
            "# extensions:\n#   slugaudit:\nextensions:\n  other:\n    cmd: /bin/other # slugaudit\n",
        ),
        (
            "crush",
            "# mcp add slugaudit --command /bin/slugaudit\necho ready # slugaudit\n",
        ),
    ] {
        let home = tempfile::tempdir().check("temp dir")?;
        let agent = find_agent(id).check("agent is registered")?;
        let config = agent.config.resolve(home.path()).check("config path")?;
        std::fs::create_dir_all(config.parent().check("config parent")?).check("mkdir")?;
        std::fs::write(&config, body).check("write commented config")?;

        ensure(
            !agent
                .is_connected(Some(home.path()))
                .check("probe commented config")?,
            format!("{id}: `#` comments mentioning slugaudit must not count as connected"),
        )?;
    }
    Ok(())
}

#[test]
fn a_malformed_json_config_for_a_json_client_is_a_probe_failure_not_connected() -> TestResult {
    let home = tempfile::tempdir().check("temp dir")?;
    let cursor = find_agent("cursor").check("cursor is registered")?;
    let config = cursor.config.resolve(home.path()).check("config path")?;
    std::fs::create_dir_all(config.parent().check("config parent")?).check("mkdir")?;
    std::fs::write(&config, "{ not json").check("write config")?;

    let (result, logs) =
        crate::store::test_capture::capture_warns(|| cursor.is_connected(Some(home.path())))?;

    let failure = result.check_err("a malformed config cannot be judged")?;
    ensure_eq(&failure.agent, &"cursor", "probe failure names cursor")?;
    ensure_eq(
        &failure.check,
        &"connected",
        "probe failure names connected check",
    )?;
    ensure_eq(&failure.path, &config, "probe failure names config path")?;
    ensure_eq(
        &failure.source.kind(),
        &std::io::ErrorKind::InvalidData,
        "probe failure preserves InvalidData kind",
    )?;
    ensure(
        failure
            .to_string()
            .starts_with("could not tell whether cursor is connected: "),
        format!("{failure}"),
    )?;
    ensure(
        !logs.contains("could not"),
        format!("the failure is returned, not logged: {logs}"),
    )?;
    Ok(())
}

#[test]
fn an_unreadable_goose_config_is_a_probe_failure_that_says_why() -> TestResult {
    let home = tempfile::tempdir().check("temp dir")?;
    let goose = find_agent("goose").check("goose is registered")?;
    let config = goose
        .config
        .resolve(home.path())
        .check("goose config path")?;
    std::fs::create_dir_all(config.parent().check("config parent")?).check("mkdir")?;
    let over_cap = usize::try_from(crate::util::MAX_FOREIGN_TEXT_BYTES)
        .check("cap fits")?
        .checked_add(1)
        .check("cap plus one fits")?;
    std::fs::write(&config, vec![b' '; over_cap]).check("write oversized config")?;

    let failure = goose
        .is_connected(Some(home.path()))
        .check_err("an oversized config cannot be judged")?;

    ensure_eq(&failure.path, &config, "probe failure names goose config")?;
    ensure(
        failure.source.to_string().contains("-byte cap"),
        format!("{failure}"),
    )?;
    Ok(())
}

#[test]
fn disconnect_fails_when_the_client_config_cannot_be_stat_ed() -> TestResult {
    let home = tempfile::tempdir().check("temp dir")?;
    std::fs::write(home.path().join(".cursor"), b"x").check("write file")?;
    let cursor = find_agent("cursor").check("cursor is registered")?;

    let result = crate::connect::disconnect_agent(&cursor, Some(home.path()));

    let Err(crate::cli::ConnectError::Io(error)) = result else {
        return Err(format!("expected Io, got {result:?}")).check("unexpected outcome");
    };
    ensure_eq(
        &error.kind(),
        &std::io::ErrorKind::NotADirectory,
        format!("the stat failure's cause must reach the caller: {error}"),
    )?;
    Ok(())
}

#[test]
fn the_installed_binary_check_fails_on_a_stat_error_instead_of_using_the_current_path() -> TestResult
{
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").check("write file")?;

    let result = temp_env::with_var("SLUGTHUG_HOME", Some(file.as_os_str()), || {
        crate::connect::prefer_slugthug_binary(std::path::Path::new("/build/artifacts/slugaudit"))
    });

    ensure(result.is_err(), format!("{result:?}"))?;
    Ok(())
}
