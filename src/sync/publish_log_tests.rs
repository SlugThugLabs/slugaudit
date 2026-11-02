//! Tests for the single mapping from a publish failure to its log fields.

use super::*;
use crate::sync::{discovery, sample};

#[test]
fn a_sampling_index_error_maps_to_its_static_category_without_a_path() {
    let error = PublishError::SamplingIndexOutOfRange { index: 7, total: 3 };

    let (error_kind, path) = failure_fields(&error);

    assert_eq!(error_kind, "sampling_index_out_of_range");
    assert_eq!(path, None);
}

/// One discovery failure of every kind, wrapped in a publish error, with
/// the log fields it must map to.
fn discovery_log_field_cases() -> Vec<(PublishError, &'static str, Option<&'static str>)> {
    use std::path::PathBuf;
    vec![
        (
            PublishError::Discovery(discovery::DiscoveryError::TimeBudgetExceeded {
                elapsed_ms: 84,
            }),
            "discovery_time_budget_exceeded",
            None,
        ),
        (
            PublishError::Discovery(discovery::DiscoveryError::NotRelative(PathBuf::from(
                "rel.rs",
            ))),
            "discovery_not_relative",
            Some("rel.rs"),
        ),
        (
            PublishError::Discovery(discovery::DiscoveryError::IgnoreRules {
                kind: "gitignore",
                path: Some(PathBuf::from("ignore.rs")),
            }),
            "discovery_ignore_rules",
            Some("ignore.rs"),
        ),
        (
            PublishError::Discovery(discovery::DiscoveryError::IgnoreRules {
                kind: "gitignore",
                path: None,
            }),
            "discovery_ignore_rules",
            None,
        ),
        (
            PublishError::Discovery(discovery::DiscoveryError::Walk(ignore::Error::WithPath {
                path: PathBuf::from("walk.rs"),
                err: Box::new(ignore::Error::from(std::io::Error::other("test"))),
            })),
            "discovery_walk",
            Some("walk.rs"),
        ),
    ]
}

/// One non-discovery publish failure of every kind, with the log fields it
/// must map to.
fn publish_log_field_cases() -> Vec<(PublishError, &'static str, Option<&'static str>)> {
    use std::path::PathBuf;
    vec![
        (
            PublishError::TimeBudgetExceeded {
                path: " a.rs".into(),
                elapsed_ms: 42,
            },
            "time_budget_exceeded",
            None,
        ),
        (
            PublishError::ChangedDuringSample {
                path: "changed.rs".into(),
            },
            "changed_during_sample",
            Some("changed.rs"),
        ),
        (
            PublishError::SamplingResultLost {
                path: "lost.rs".into(),
                reason: "channel closed".into(),
            },
            "sampling_result_lost",
            Some("lost.rs"),
        ),
        (
            PublishError::Revision(RevisionError::InvalidParserRun {
                path: "run.rs".into(),
                reason: "err",
            }),
            "revision_invalid_parser_run",
            Some("run.rs"),
        ),
        (
            PublishError::Sample(sample::SampleError::Read {
                path: PathBuf::from("sample.rs"),
                source: std::io::Error::other("test"),
            }),
            "sample_read",
            Some("sample.rs"),
        ),
        (
            PublishError::Sample(sample::SampleError::NotUtf8 {
                path: PathBuf::from("bytes.rs"),
            }),
            "sample_not_utf8",
            Some("bytes.rs"),
        ),
        (
            PublishError::Limits(crate::project::ProjectLimitsError::Config(
                crate::project::ConfigError::Io {
                    path: PathBuf::from("config.json"),
                    source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
                },
            )),
            "project_limits",
            None,
        ),
        (
            PublishError::Database(rusqlite::Error::QueryReturnedNoRows),
            "database",
            None,
        ),
        (
            PublishError::ImportTooLarge {
                total: 10,
                limit: 5,
            },
            "import_too_large",
            None,
        ),
        (
            PublishError::SamplingIndexOutOfRange { index: 2, total: 1 },
            "sampling_index_out_of_range",
            None,
        ),
        (
            PublishError::InvalidParserPackVersion("v0".into()),
            "invalid_parser_pack_version",
            None,
        ),
    ]
}

#[test]
fn every_publish_failure_maps_to_its_kind_and_path() {
    let mut cases = discovery_log_field_cases();
    cases.extend(publish_log_field_cases());
    for (error, kind, path) in &cases {
        assert_eq!(
            failure_fields(error),
            (*kind, path.map(Path::new)),
            "log fields of {error:?}"
        );
    }
}
