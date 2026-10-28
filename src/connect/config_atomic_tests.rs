//! Mode preservation and key-order tests for the config writers.

use super::*;
use crate::connect::config_file::write_json_server;

#[cfg(unix)]
#[test]
fn config_file_replacing_a_0644_config_preserves_its_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().expect("temp dir");
    let config = home.path().join("mcp.json");
    std::fs::write(&config, "{}").expect("write config");
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    write_atomic(&config, b"{\"a\":1}\n").expect("replace");

    let mode = std::fs::metadata(&config)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644, "an existing mode is not tightened or widened");
}

#[cfg(unix)]
#[test]
fn config_file_new_file_is_created_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = tempfile::tempdir().expect("temp dir");
    let config = home.path().join("fresh.json");

    write_atomic(&config, b"{}\n").expect("create");

    let mode = std::fs::metadata(&config)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "a new config is owner-only");
}

#[test]
fn config_file_round_trip_keeps_key_order_and_ends_with_newline() {
    let home = tempfile::tempdir().expect("temp dir");
    let config = home.path().join("mcp.json");
    std::fs::write(
        &config,
        r#"{"zeta":1,"mcpServers":{"zz":{},"aa":{}},"alpha":2}"#,
    )
    .expect("write config");

    write_json_server(&config, "mcpServers", serde_json::json!({"command": "x"})).expect("write");

    let text = std::fs::read_to_string(&config).expect("read back");
    let position = |needle: &str| text.find(needle).expect("key present");
    assert!(position("\"zeta\"") < position("\"mcpServers\""));
    assert!(position("\"mcpServers\"") < position("\"alpha\""));
    assert!(position("\"zz\"") < position("\"aa\""));
    assert!(position("\"aa\"") < position("\"slugaudit\""));
    assert!(text.ends_with("}\n"), "trailing newline: {text:?}");
}
