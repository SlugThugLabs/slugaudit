//! Tests for the logging setup in `main.rs`.

use super::*;

#[test]
fn unset_log_variables_take_their_defaults_without_a_complaint() {
    let settings = log_settings(
        Err(std::env::VarError::NotPresent),
        Err(std::env::VarError::NotPresent),
    );

    assert!(!settings.json);
    assert!(settings.rejected.is_empty());
}

#[test]
fn an_invalid_rust_log_is_reported_not_dropped() {
    let settings = log_settings(
        Ok("slugaudit=trace,,=bogus[".to_owned()),
        Err(std::env::VarError::NotPresent),
    );

    assert_eq!(settings.rejected.len(), 1, "{:?}", settings.rejected);
    assert!(settings.rejected[0].contains("RUST_LOG"));
}

#[test]
fn a_valid_rust_log_and_json_format_are_honored_silently() {
    let settings = log_settings(Ok("debug".to_owned()), Ok("JSON".to_owned()));

    assert!(settings.json);
    assert!(settings.rejected.is_empty());
}

#[cfg(unix)]
#[test]
fn a_non_utf8_log_format_is_reported_not_treated_as_plain() {
    use std::os::unix::ffi::OsStringExt as _;
    let bad = std::ffi::OsString::from_vec(b"SLUGAUDIT_LEAK_MARKER\xff".to_vec());

    let settings = log_settings(
        Err(std::env::VarError::NotPresent),
        Err(std::env::VarError::NotUnicode(bad)),
    );

    assert!(!settings.json);
    assert_eq!(settings.rejected.len(), 1);
    assert!(settings.rejected[0].contains("SLUGAUDIT_LOG_FORMAT"));
    assert!(
        !settings.rejected[0].contains("SLUGAUDIT_LEAK_MARKER"),
        "the raw environment value must not reach the log: {:?}",
        settings.rejected[0]
    );
}

#[cfg(unix)]
#[test]
fn a_non_utf8_rust_log_is_reported_without_echoing_its_value() {
    use std::os::unix::ffi::OsStringExt as _;
    let bad = std::ffi::OsString::from_vec(b"RUST_LOG_LEAK_MARKER\xff".to_vec());

    let settings = log_settings(
        Err(std::env::VarError::NotUnicode(bad)),
        Err(std::env::VarError::NotPresent),
    );

    assert_eq!(settings.rejected.len(), 1);
    assert!(settings.rejected[0].contains("RUST_LOG"));
    assert!(
        !settings.rejected[0].contains("RUST_LOG_LEAK_MARKER"),
        "the raw environment value must not reach the log: {:?}",
        settings.rejected[0]
    );
}

/// A `Write` that appends into a shared buffer, for capturing log output.
#[derive(Clone)]
struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("sink lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn honest_log_default_filter_keeps_our_info_and_drops_rmcp_info() {
    let settings = log_settings(
        Err(std::env::VarError::NotPresent),
        Err(std::env::VarError::NotPresent),
    );
    assert!(settings.rejected.is_empty());
    let sink = Sink(std::sync::Arc::default());
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_env_filter(settings.filter)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "slugaudit_mcp_rust::server_runner", "ours-info-marker");
        tracing::info!(target: "rmcp::service", "rmcp-info-marker");
        tracing::warn!(target: "rmcp::service", "rmcp-warn-marker");
        tracing::info!(target: "hyper::proto", "dep-info-marker");
        tracing::warn!(target: "hyper::proto", "dep-warn-marker");
    });

    let out =
        String::from_utf8(sink.0.lock().expect("sink lock").clone()).expect("log output is utf-8");
    assert!(out.contains("ours-info-marker"), "{out}");
    assert!(!out.contains("rmcp-info-marker"), "{out}");
    assert!(!out.contains("rmcp-warn-marker"), "{out}");
    assert!(!out.contains("dep-info-marker"), "{out}");
    assert!(out.contains("dep-warn-marker"), "{out}");
}
