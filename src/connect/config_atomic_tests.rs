//! Mode preservation and key-order tests for the config writers.

use super::*;
use crate::connect::config_file::write_json_server;
use slugaudit_test_check::{Check, TestResult};

#[cfg(unix)]
#[test]
fn config_file_replacing_a_0644_config_preserves_its_mode() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().check("temp dir")?;
    let config = home.path().join("mcp.json");
    std::fs::write(&config, "{}").check("write config")?;
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).check("chmod")?;

    write_atomic(&config, b"{\"a\":1}\n").check("replace")?;

    let mode = std::fs::metadata(&config)
        .check("stat")?
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644, "an existing mode is not tightened or widened");
    Ok(())
}

#[cfg(unix)]
#[test]
fn config_file_new_file_is_created_owner_only() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().check("temp dir")?;
    let config = home.path().join("fresh.json");

    write_atomic(&config, b"{}\n").check("create")?;

    let mode = std::fs::metadata(&config)
        .check("stat")?
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "a new config is owner-only");
    Ok(())
}

#[test]
fn config_file_round_trip_keeps_key_order_and_ends_with_newline() -> TestResult {
    let home = tempfile::tempdir().check("temp dir")?;
    let config = home.path().join("mcp.json");
    std::fs::write(
        &config,
        r#"{"zeta":1,"mcpServers":{"zz":{},"aa":{}},"alpha":2}"#,
    )
    .check("write config")?;

    write_json_server(&config, "mcpServers", serde_json::json!({"command": "x"})).check("write")?;

    let text = std::fs::read_to_string(&config).check("read back")?;
    let position = |needle: &str| text.find(needle).check("key present");
    assert!(position("\"zeta\"")? < position("\"mcpServers\"")?);
    assert!(position("\"mcpServers\"")? < position("\"alpha\"")?);
    assert!(position("\"zz\"")? < position("\"aa\"")?);
    assert!(position("\"aa\"")? < position("\"slugaudit\"")?);
    assert!(text.ends_with("}\n"), "trailing newline: {text:?}");
    Ok(())
}
