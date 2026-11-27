//! Unit and end-to-end tests for JSONC (`opencode.jsonc`) comment detection,
//! quote-aware block-comment stripping, and comment-preserving write refusals.

use super::*;
use crate::connect::{
    ConnectError, DisconnectOutcome, connect_agent, disconnect_agent, find_agent,
};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use std::path::Path;

#[test]
fn jsonc_normalizer_preserves_urls_and_strips_comments_and_trailing_commas() -> TestResult {
    let raw = "{\n  // Top-level line comment\n  \"$schema\": \"https://opencode.ai/config.json\",\n  /* Multi-line\n     block comment */\n  \"note\": \"keep /* comment-like */ and \\\"//escaped\\\" text,\",\n  \"mcp\": {\n    \"docs\": {\n      \"type\": \"remote\",\n      \"url\": \"https://example.com/mcp?x=1,\",\n      \"headers\": [\"a\", \"b\",], // trailing comma before ]\n    }, /* trailing comma before } */\n  },\n}\n";

    let cleaned = strip_jsonc_comments_and_trailing_commas(raw);
    let parsed: serde_json::Value =
        serde_json::from_str(&cleaned).check("normalized JSONC parses cleanly")?;

    ensure_eq(
        &parsed.get("$schema").and_then(serde_json::Value::as_str),
        &Some("https://opencode.ai/config.json"),
        "URL with // inside string literal is preserved intact",
    )?;
    ensure_eq(
        &parsed.get("note").and_then(serde_json::Value::as_str),
        &Some("keep /* comment-like */ and \"//escaped\" text,"),
        "comment-like tokens and escaped quotes inside strings survive untouched",
    )?;
    ensure_eq(
        &parsed
            .pointer("/mcp/docs/url")
            .and_then(serde_json::Value::as_str),
        &Some("https://example.com/mcp?x=1,"),
        "nested URL with trailing comma inside quotes is preserved",
    )?;
    Ok(())
}

#[test]
fn jsonc_block_comments_handle_quoted_close_markers_and_strings_handle_comment_openers()
-> TestResult {
    let raw = "{\n  /* block comment quoting \"a */ b\" and \"escaped \\\" */ end\" */\n  \"literal_open\": \"/* not a comment\",\n  \"escaped_open\": \"quote \\\" /* still in string\",\n}\n";
    ensure(
        has_jsonc_comments(raw),
        "detects real block comment outside strings",
    )?;
    ensure(
        !has_jsonc_comments(
            "{\"url\": \"https://opencode.ai/config.json\", \"glob\": \"src/*\", \"esc\": \"\\\"//nope\"}",
        ),
        "comment-like markers inside strings are not flagged as comments",
    )?;

    let cleaned = strip_jsonc_comments_and_trailing_commas(raw);
    let parsed: serde_json::Value = serde_json::from_str(&cleaned)
        .check("quoted */ inside block comment does not cut early")?;
    ensure_eq(
        &parsed
            .get("literal_open")
            .and_then(serde_json::Value::as_str),
        &Some("/* not a comment"),
        "preserves /* inside string literal",
    )?;
    ensure_eq(
        &parsed
            .get("escaped_open")
            .and_then(serde_json::Value::as_str),
        &Some("quote \" /* still in string"),
        "preserves /* after escaped quote inside string literal",
    )?;
    Ok(())
}

#[test]
fn jsonc_normalizer_leaves_unclosed_block_comments_and_bare_commas_invalid() -> TestResult {
    for invalid in [
        "{ /* unclosed block comment ",
        "{ /* unclosed before quoted close \n \"x\": \"a */ b\" }",
        "{,}",
        "{\"a\": [,]}",
        "{\"a\": [1,,]}",
    ] {
        let cleaned = strip_jsonc_comments_and_trailing_commas(invalid);
        ensure(
            serde_json::from_str::<serde_json::Value>(&cleaned).is_err(),
            format!("malformed JSONC {invalid:?} must still fail parsing"),
        )?;
    }
    Ok(())
}

#[test]
fn opencode_jsonc_refuses_to_overwrite_existing_comments_and_handles_trailing_commas() -> TestResult
{
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let empty_bin = tempfile::tempdir().check("empty bin dir")?;
    let home = tempfile::tempdir().check("temp home")?;
    let opencode = find_agent("opencode").check("opencode is registered")?;
    let config = opencode
        .config
        .resolve(home.path())
        .check("opencode config path")?;
    std::fs::create_dir_all(config.parent().check("parent")?).check("mkdir opencode")?;

    let commented = "{\n  // OpenCode user configuration\n  \"$schema\": \"https://opencode.ai/config.json\",\n  /* Existing server */\n  \"mcp\": {\n    \"existing\": {\n      \"type\": \"local\",\n      \"command\": [\"/usr/local/bin/other-mcp\",],\n    },\n  },\n}\n";
    std::fs::write(&config, commented).check("seed commented opencode.jsonc")?;

    temp_env::with_var(
        "PATH",
        Some(empty_bin.path().as_os_str()),
        || -> TestResult {
            ensure(
                !opencode
                    .is_connected(Some(home.path()))
                    .check("probe commented opencode.jsonc")?,
                "read-only probe parses commented jsonc without modifying it",
            )?;

            let noop = disconnect_agent(&opencode, Some(home.path())).check("noop disconnect")?;
            ensure_eq(
                &noop,
                &DisconnectOutcome::NothingToDo,
                "absent slugaudit entry reports NothingToDo",
            )?;

            let err = connect_agent(&opencode, Path::new("/bin/slugaudit"), Some(home.path()))
                .check_err("connect must refuse to erase existing jsonc comments")?;
            let ConnectError::CommentedConfig { path, advice } = err else {
                return Err(format!("expected CommentedConfig, got {err:?}"))
                    .check("unexpected error variant");
            };
            ensure_eq(&path, &config, "CommentedConfig names opencode.jsonc")?;
            ensure(
                advice.contains("\"mcp\"") && advice.contains("\"slugaudit\""),
                format!("advice names mcp section and slugaudit entry: {advice}"),
            )?;
            ensure_eq(
                &std::fs::read_to_string(&config).check("read preserved jsonc")?,
                &commented,
                "refusal leaves user comments byte-for-byte untouched",
            )?;

            // Uncommented `.jsonc` (with URL `//` inside strings and trailing commas)
            // rewrites cleanly on connect and disconnect.
            let uncommented = "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {\n    \"existing\": {\n      \"type\": \"local\",\n      \"command\": [\"/usr/local/bin/other-mcp\",],\n    },\n  },\n}\n";
            std::fs::write(&config, uncommented).check("write uncommented jsonc")?;

            connect_agent(&opencode, Path::new("/bin/slugaudit"), Some(home.path()))
                .check("connect opencode on uncommented jsonc")?;
            ensure(
                opencode
                    .is_connected(Some(home.path()))
                    .check("probe connected opencode.jsonc")?,
                "opencode reports connected after fallback write",
            )?;

            let removed = disconnect_agent(&opencode, Some(home.path()))
                .check("disconnect opencode from uncommented jsonc")?;
            ensure_eq(
                &removed,
                &DisconnectOutcome::Disconnected,
                "disconnect removes slugaudit from uncommented jsonc",
            )?;
            ensure(
                !opencode
                    .is_connected(Some(home.path()))
                    .check("probe after disconnect")?,
                "opencode reports disconnected after removal",
            )?;
            Ok(())
        },
    )
}
