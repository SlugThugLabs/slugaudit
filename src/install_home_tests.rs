//! Tests for `install` home-directory discovery from the `SLUGTHUG_HOME`,
//! `SLUGTHUG_CONFIG_HOME` and `HOME` variables.
use super::*;
use slugaudit_test_check::{TestResult, ensure_eq};

#[test]
fn slugthug_home_prefers_slugthug_home_over_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(temp.path().as_os_str())),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &slugthug_home().check("home resolves")?,
                &temp.path(),
                "slugthug_home().check(\"home resolves\")? == temp.path()",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn slugaudit_dir_resolves_under_slugthug_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    temp_env::with_var(
        "SLUGTHUG_HOME",
        Some(temp.path().as_os_str()),
        || -> TestResult {
            ensure_eq(
                &slugaudit_dir().check("slugaudit dir resolves")?,
                &temp.path().join("slugaudit"),
                "slugaudit_dir().check(\"slugaudit dir resolves\")? == temp.path().join(\"slugaudit\")",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn config_home_is_the_user_home_not_the_slugthug_directory() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(std::ffi::OsStr::new("/slug/home"))),
            ("SLUGTHUG_CONFIG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &config_home().check("config home resolves")?,
                &PathBuf::from("/somewhere/else"),
                "client config must resolve against the user's home, never ~/.slugthug",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn config_home_override_wins_over_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            (
                "SLUGTHUG_CONFIG_HOME",
                Some(std::ffi::OsStr::new("/agents/home")),
            ),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &config_home().check("config home resolves")?,
                &PathBuf::from("/agents/home"),
                "config_home().check(\"config home resolves\")? == PathBuf::from(\"/agents/home\")",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn a_client_config_path_resolves_under_the_user_home() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(std::ffi::OsStr::new("/slug/home"))),
            ("SLUGTHUG_CONFIG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &config_home()
                    .check("config home resolves")?
                    .join(".cursor/mcp.json"),
                &PathBuf::from("/somewhere/else/.cursor/mcp.json"),
                "guards the regression where a client config landed in ~/.slugthug/ and the client never read it",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn slugthug_home_falls_back_to_home_slugthug() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &slugthug_home().check("home resolves")?,
                &PathBuf::from("/somewhere/else/.slugthug"),
                "slugthug_home().check(\"home resolves\")? == PathBuf::from(\"/somewhere/else/.slugthug\")",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn slugthug_home_is_none_without_either_var() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", None::<&std::ffi::OsStr>),
            ("HOME", None::<&std::ffi::OsStr>),
        ],
        || ensure_eq(&slugthug_home(), &None, "slugthug_home() == None"),
    )?;
    Ok(())
}

#[test]
fn empty_home_variables_count_as_unset() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let empty = std::ffi::OsStr::new("");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(empty)),
            ("SLUGTHUG_CONFIG_HOME", Some(empty)),
            ("HOME", Some(empty)),
        ],
        || -> TestResult {
            ensure_eq(&slugthug_home(), &None, "slugthug_home() == None")?;
            ensure_eq(&slugaudit_dir(), &None, "slugaudit_dir() == None")?;
            ensure_eq(&config_home(), &None, "config_home() == None")?;
            Ok(())
        },
    )?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(empty)),
            ("SLUGTHUG_CONFIG_HOME", Some(empty)),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &slugthug_home(),
                &Some(PathBuf::from("/somewhere/else/.slugthug")),
                "slugthug_home() == Some(PathBuf::from(\"/somewhere/else/.slugthug\"))",
            )?;
            ensure_eq(
                &config_home(),
                &Some(PathBuf::from("/somewhere/else")),
                "config_home() == Some(PathBuf::from(\"/somewhere/else\"))",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn relative_home_variables_count_as_unset() -> TestResult {
    let _guard = TEST_ENV_LOCK.lock().check("env lock")?;
    let relative = std::ffi::OsStr::new("relative/home");
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(relative)),
            ("SLUGTHUG_CONFIG_HOME", Some(relative)),
            ("HOME", Some(relative)),
        ],
        || -> TestResult {
            ensure_eq(&slugthug_home(), &None, "relative SLUGTHUG_HOME rejected")?;
            ensure_eq(&slugaudit_dir(), &None, "relative slugaudit_dir rejected")?;
            ensure_eq(&config_home(), &None, "relative config_home rejected")?;
            Ok(())
        },
    )?;
    temp_env::with_vars(
        [
            ("SLUGTHUG_HOME", Some(relative)),
            ("SLUGTHUG_CONFIG_HOME", Some(relative)),
            ("HOME", Some(std::ffi::OsStr::new("/somewhere/else"))),
        ],
        || -> TestResult {
            ensure_eq(
                &slugthug_home(),
                &Some(PathBuf::from("/somewhere/else/.slugthug")),
                "falls back to absolute HOME when SLUGTHUG_HOME is relative",
            )?;
            ensure_eq(
                &config_home(),
                &Some(PathBuf::from("/somewhere/else")),
                "falls back to absolute HOME when SLUGTHUG_CONFIG_HOME is relative",
            )?;
            Ok(())
        },
    )?;
    Ok(())
}
