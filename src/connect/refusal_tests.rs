//! Tests for the configs and entries `connect` refuses to write, and the
//! remedy each refusal prints.

use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[test]
fn json_agent_refuses_to_overwrite_invalid_or_commented_json() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let agent = find_agent("cursor").check("cursor agent")?;
    let binary = Path::new("/bin/slugaudit");

    let cfg_path = home.join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().check("parent")?).check("mkdir")?;
    let original = "{\n  // User comment\n  \"mcpServers\": {}\n}";
    fs::write(&cfg_path, original).check("write original")?;

    let result = connect_agent(&agent, binary, Some(home));
    ensure(
        matches!(result, Err(ConnectError::InvalidConfig { .. })),
        format!("must error on invalid json, got {result:?}"),
    )?;
    let after = fs::read_to_string(&cfg_path).check("read after")?;
    ensure_eq(&after, &original, "original file must remain untouched")?;
    Ok(())
}

#[test]
fn connect_refuses_entries_it_cannot_write_safely() -> TestResult {
    let cases: &[(&str, Option<&str>)] = &[
        ("goose", Some(".config/goose/config.yaml")),
        ("crush", Some(".config/crush/crushrc")),
    ];
    for (id, file) in cases {
        let temp = tempfile::tempdir().check("tempdir")?;
        let home = temp.path();
        let agent = find_agent(id).check("agent")?;
        let err = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(home))
            .check_err("connect must refuse")?;
        match err {
            ConnectError::UnsupportedAgent {
                agent: name,
                reason,
            } => {
                ensure_eq(
                    &name,
                    &agent.display_name,
                    "refusal names display_name of manual agent",
                )?;
                ensure(
                    reason.contains("no automated write is possible"),
                    format!("{id} must say the write cannot proceed: {reason}"),
                )?;
                ensure(
                    reason.contains(&agent.remedy.replace("{binary}", "/bin/slugaudit")),
                    format!("{id} must print its remedy: {reason}"),
                )?;
            }
            other => {
                return Err(format!(
                    "{id} must refuse automated registration, got {other:?}"
                ))
                .check("unexpected outcome");
            }
        }
        if let Some(file) = file {
            ensure(
                !home.join(file).exists(),
                format!("{id} must not write {file}"),
            )?;
        }
    }
    Ok(())
}

#[test]
fn goose_refusal_prints_the_paste_ready_yaml_block() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let home = temp.path();
    let goose = find_agent("goose").check("goose")?;

    ensure(
        ConfigShape::GooseExtensions.write_section().is_none(),
        "the YAML shape must not be machine-written",
    )?;
    ensure_eq(
        &ConfigShape::GooseExtensions.sections(),
        &["extensions"],
        "detection still reads the extensions key",
    )?;

    let err = connect_agent(&goose, Path::new("/bin/slugaudit"), Some(home))
        .check_err("connect must refuse")?;
    let ConnectError::UnsupportedAgent { reason, .. } = err else {
        return Err("expected an UnsupportedAgent refusal".to_owned()).check("unexpected outcome");
    };
    ensure(
        reason.contains(".config/goose/config.yaml"),
        format!("the refusal must name the target file: {reason}"),
    )?;
    ensure(
        reason.contains(
            "extensions:\n  slugaudit:\n    type: stdio\n    cmd: /bin/slugaudit\n    args: []",
        ),
        format!("the refusal must carry the paste-ready block: {reason}"),
    )?;
    ensure(
        !home.join(".config/goose/config.yaml").exists(),
        "goose's write path must be unreachable",
    )?;
    Ok(())
}

/// A config whose top level is not an object has nowhere to hold an MCP
/// section; the refusal names the file, not an unrelated missing home.
#[test]
fn a_json_config_whose_top_level_is_not_an_object_is_refused_by_path() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let agent = find_agent("cursor").check("cursor agent")?;
    let cfg_path = temp.path().join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().check("parent")?).check("mkdir")?;
    fs::write(&cfg_path, "[]").check("write array config")?;

    let result = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(temp.path()));

    let Err(ConnectError::RootNotAnObject { path }) = result else {
        return Err(format!("expected RootNotAnObject, got {result:?}"))
            .check("unexpected outcome");
    };
    ensure_eq(&path, &cfg_path, "the refusal names the config file")?;
    ensure_eq(
        &fs::read_to_string(&cfg_path).check("read")?,
        &"[]",
        "file untouched",
    )?;
    Ok(())
}

/// A section key holding a non-object is refused rather than rewritten
/// unchanged under a "Wrote" message.
#[test]
fn a_json_config_whose_server_section_is_not_an_object_is_refused_by_section() -> TestResult {
    let temp = tempfile::tempdir().check("tempdir")?;
    let agent = find_agent("cursor").check("cursor agent")?;
    let cfg_path = temp.path().join(".cursor/mcp.json");
    fs::create_dir_all(cfg_path.parent().check("parent")?).check("mkdir")?;
    fs::write(&cfg_path, r#"{"mcpServers": []}"#).check("write config")?;

    let result = connect_agent(&agent, Path::new("/bin/slugaudit"), Some(temp.path()));

    let Err(ConnectError::SectionNotAnObject { section }) = result else {
        return Err(format!("expected SectionNotAnObject, got {result:?}"))
            .check("unexpected outcome");
    };
    ensure_eq(&section, &"mcpServers", "refusal names non-object section")?;
    Ok(())
}
