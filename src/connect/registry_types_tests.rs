//! Unit tests for `registry_types`: `%APPDATA%` expansion and non-JSON shapes.

use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn the_appdata_token_expands_from_the_environment_and_is_never_guessed() -> TestResult {
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let home = Path::new("/home/u");

    let set = temp_env::with_var("APPDATA", Some("/roaming"), || {
        expand_os_tokens("%APPDATA%/Claude/c.json", home)
    });
    let unset = temp_env::with_var_unset("APPDATA", || {
        expand_os_tokens("%APPDATA%/Claude/c.json", home)
    });
    let empty = temp_env::with_var("APPDATA", Some(""), || {
        expand_os_tokens("%APPDATA%/Claude/c.json", home)
    });

    ensure_eq(
        &set,
        &Some(PathBuf::from("/roaming/Claude/c.json")),
        "expands %APPDATA% when variable is set",
    )?;
    ensure_eq(
        &unset,
        &None,
        "an unset APPDATA must not become home/AppData",
    )?;
    ensure_eq(
        &empty,
        &None,
        "an empty APPDATA must not produce a relative path",
    )?;
    ensure_eq(
        &expand_os_tokens(".config/x.json", home),
        &Some(PathBuf::from("/home/u/.config/x.json")),
        "joins plain home-relative path onto home",
    )?;
    Ok(())
}

#[test]
fn shapes_without_a_json_writer_have_no_entry() -> TestResult {
    for shape in [ConfigShape::GooseExtensions, ConfigShape::Unknown] {
        let result = shape.entry(Path::new("/bin/slugaudit"));
        ensure(
            matches!(result, Err(ConnectError::UnsupportedAgent { .. })),
            format!("{shape:?}: {result:?}"),
        )?;
    }
    Ok(())
}
