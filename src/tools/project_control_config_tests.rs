use super::*;
use std::path::PathBuf;

#[test]
fn a_config_write_io_failure_is_an_internal_error_naming_the_cause() {
    let error = project::ConfigWriteError::Io {
        path: PathBuf::from("/project/.planning/slugaudit/config.json"),
        source: std::io::Error::other("no space left on device"),
    };

    let mapped = config_write_error(error);

    assert_eq!(
        mapped.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{mapped:?}"
    );
    assert!(
        mapped.message.contains("write profile config")
            && mapped.message.contains("no space left on device"),
        "the message keeps the operation and the source error: {}",
        mapped.message
    );
}

#[test]
fn a_non_regular_config_target_is_invalid_params() {
    let error = project::ConfigWriteError::NotRegularFile {
        path: PathBuf::from("/project/.planning/slugaudit/config.json"),
    };

    let mapped = config_write_error(error);

    assert_eq!(
        mapped.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "{mapped:?}"
    );
    assert!(mapped.message.contains("config.json"), "{}", mapped.message);
}
