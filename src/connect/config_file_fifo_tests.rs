//! A FIFO at a client's config path is never opened: opening one blocks
//! until a writer appears, so `connect` and connection detection decide from
//! metadata alone and leave it untouched.

use super::*;

/// `mkfifo(1)` creates the fixture because std exposes no safe FIFO
/// constructor and `unsafe` is forbidden; it exists on both supported targets.
#[cfg(unix)]
fn create_fifo(path: &Path) -> TestResult {
    let mut mkfifo = "mkfifo";
    for candidate in ["/usr/bin/mkfifo", "/bin/mkfifo"] {
        if Path::new(candidate).exists() {
            mkfifo = candidate;
            break;
        }
    }
    let status = std::process::Command::new(mkfifo)
        .arg(path)
        .status()
        .check("run mkfifo")?;
    assert!(status.success(), "mkfifo creates the fixture: {status}");
    Ok(())
}

/// Runs `f` on a thread so a regression to a blocking open fails the test
/// instead of hanging the whole suite.
#[cfg(unix)]
fn within_ten_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> TestResult<T> {
    let (sender, receiver) = std::sync::mpsc::channel();
    // A failed send means the receiver already timed out and failed the test.
    std::thread::spawn(move || sender.send(f()));
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .check("the config probe must not block on a non-regular file")
}

/// Opening a FIFO blocks until a writer appears, so a connect must refuse it
/// from metadata alone and leave it untouched.
#[cfg(unix)]
#[test]
fn a_fifo_config_is_refused_by_a_connect_without_blocking() -> TestResult {
    use std::os::unix::fs::FileTypeExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".cursor")).check("cursor dir")?;
    let config = home.join(".cursor/mcp.json");
    create_fifo(&config)?;
    let cursor = find_agent("cursor").check("cursor is registered")?;
    let probe_home = home.clone();

    let result = within_ten_seconds(move || {
        connect_agent(&cursor, Path::new("/bin/slugaudit"), Some(&probe_home))
    })?;

    let Err(ConnectError::Io(error)) = result else {
        return Err(format!("expected Io, got {result:?}")).check("unexpected outcome");
    };
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidInput,
        "the FIFO must be refused before the read: {error}"
    );
    assert!(
        error.to_string().contains("not a regular file"),
        "the refusal must name the file type: {error}"
    );
    assert!(
        config
            .symlink_metadata()
            .check("stat the FIFO")?
            .file_type()
            .is_fifo(),
        "the FIFO is left untouched"
    );
    Ok(())
}

/// Detection probes the config path too, so cursor and goose must both judge a
/// FIFO from metadata instead of opening it.
#[cfg(unix)]
#[test]
fn a_fifo_config_is_not_opened_by_connection_detection() -> TestResult {
    use std::os::unix::fs::FileTypeExt as _;
    let _guard = crate::util::TEST_ENV_LOCK.lock().check("env lock")?;
    let temp = tempfile::tempdir().check("temp dir")?;
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".cursor")).check("cursor dir")?;
    std::fs::create_dir_all(home.join(".config/goose")).check("goose dir")?;
    let cursor_fifo = home.join(".cursor/mcp.json");
    let goose_fifo = home.join(".config/goose/config.yaml");
    create_fifo(&cursor_fifo)?;
    create_fifo(&goose_fifo)?;
    let empty = tempfile::tempdir().check("empty bin dir")?;
    let cursor = find_agent("cursor").check("cursor is registered")?;
    let goose = find_agent("goose").check("goose is registered")?;
    let cursor_home = home.clone();
    let goose_home = home.clone();

    let (cursor_result, goose_result) = temp_env::with_var(
        "PATH",
        Some(empty.path().as_os_str()),
        || -> TestResult<_> {
            Ok((
                within_ten_seconds(move || cursor.is_connected(Some(&cursor_home)))?,
                within_ten_seconds(move || goose.is_connected(Some(&goose_home)))?,
            ))
        },
    )?;

    for (id, result) in [("cursor", cursor_result), ("goose", goose_result)] {
        let failure = result.check_err("a FIFO config cannot be examined")?;
        assert_eq!(failure.agent, id);
        assert_eq!(failure.check, "connected");
        assert_eq!(failure.source.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            failure.source.to_string().contains("not a regular file"),
            "{failure}"
        );
    }
    for fifo in [&cursor_fifo, &goose_fifo] {
        assert!(
            fifo.symlink_metadata()
                .check("stat the FIFO")?
                .file_type()
                .is_fifo(),
            "the FIFO is left untouched"
        );
    }
    let error = readable_config(&cursor_fifo).check_err("a FIFO is refused, not read")?;
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    Ok(())
}
