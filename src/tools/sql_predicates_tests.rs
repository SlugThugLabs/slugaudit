//! Unit tests for [`super`] (`CREDENTIAL_PATH_PREDICATE`, `pattern_to_glob`).

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

fn credential_path_predicate_matches(path: &str) -> TestResult<bool> {
    let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;
    let matched: i64 = connection
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM (SELECT ?1 AS path) WHERE {CREDENTIAL_PATH_PREDICATE})"
            ),
            rusqlite::params![path],
            |row| row.get(0),
        )
        .check("evaluate credential predicate")?;
    Ok(matched != 0)
}

fn assert_predicate_cases(cases: &[(&str, bool)]) -> TestResult {
    for &(path, expected) in cases {
        ensure_eq(
            &credential_path_predicate_matches(path)?,
            &expected,
            format!("predicate verdict for {path}"),
        )?;
    }
    Ok(())
}

#[test]
fn credential_path_predicate_matches_credential_shaped_paths() -> TestResult {
    assert_predicate_cases(&[
        (".env", true),
        (".env.local", true),
        ("config/.env", true),
        (".env.production", true),
        (".ENV", true),
        ("server.pem", true),
        ("keys/cred.pem", true),
        ("keys/tls.key", true),
        ("main.key", true),
        ("cert.p12", true),
        ("cert.pfx", true),
        (".netrc", true),
        ("home/.netrc", true),
        (".npmrc", true),
        ("proj/.npmrc", true),
        (".git-credentials", true),
        ("home/.git-credentials", true),
        ("AuthKey_ABC123.p8", true),
        ("debug.jks", true),
        ("release.keystore", true),
        ("terraform.tfstate", true),
        ("prod.terraform.tfstate", true),
        ("service-account.json", true),
        ("keys/service-account.json", true),
        ("service-account-prod.json", true),
    ])?;
    Ok(())
}

#[test]
fn credential_path_predicate_matches_ssh_private_keys() -> TestResult {
    assert_predicate_cases(&[
        ("id_rsa", true),
        ("id_ed25519", true),
        ("id_ecdsa", true),
        ("id_dsa", true),
        (".ssh/id_rsa", true),
        ("home/.ssh/id_ecdsa", true),
        ("backup/id_rsa_bak", true),
        ("ID_RSA", true),
    ])?;
    Ok(())
}

#[test]
fn credential_path_predicate_rejects_public_keys_and_lookalikes() -> TestResult {
    assert_predicate_cases(&[
        ("id_rsa.pub", false),
        ("id_ed25519.pub", false),
        (".ssh/id_rsa.pub", false),
        (".ssh/id_ecdsa.pub", false),
        ("sub/id-rsa", false),
        ("sub/idXrsa", false),
        ("keyboard.rs", false),
        ("notes.key.md", false),
        ("pem_util.rs", false),
        ("keystore.rs", false),
        ("src/lib.rs", false),
        ("README.md", false),
        ("environment.rs", false),
        (".envrc", false),
        ("authorized_keys", false),
        ("known_hosts", false),
    ])?;
    Ok(())
}

#[test]
fn pattern_to_glob_collapses_globstars_and_escapes_literal_brackets() -> TestResult {
    ensure_eq(
        &pattern_to_glob("src/**/*.rs"),
        &"src/*.rs",
        "middle globstar collapses to single slash wildcard",
    )?;
    ensure_eq(
        &pattern_to_glob("**/lib.rs"),
        &"lib.rs".to_owned(),
        "leading **/ strips cleanly",
    )?;
    ensure_eq(
        &pattern_to_glob("users/[slug]"),
        &"*users/[[]slug]*",
        "substring mode escapes literal open bracket",
    )?;
    Ok(())
}
