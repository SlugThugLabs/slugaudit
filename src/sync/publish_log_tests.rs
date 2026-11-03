//! Tests for the single mapping from a publish failure to its log fields.

use super::*;
use crate::sync::{discovery, sample};
use slugaudit_test_check::{CheckErr, TestResult, ensure_eq};

#[test]
fn a_sampling_index_error_maps_to_its_static_category_without_a_path() -> TestResult {
    let error = PublishError::SamplingIndexOutOfRange { index: 7, total: 3 };

    let (error_kind, path) = failure_fields(&error);

    ensure_eq(
        &error_kind,
        &"sampling_index_out_of_range",
        "error_kind == \"sampling_index_out_of_range\"",
    )?;
    ensure_eq(&path, &None, "path == None")?;
    Ok(())
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
fn publish_log_field_cases() -> TestResult<Vec<(PublishError, &'static str, Option<&'static str>)>>
{
    use std::path::PathBuf;
    let invalid = vec![0xff_u8];
    let not_utf8 = std::str::from_utf8(&invalid).check_err("0xff is not valid UTF-8")?;
    Ok(vec![
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
                source: not_utf8,
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
            PublishError::ImportTotalOverflow { limit: 5 },
            "import_total_overflow",
            None,
        ),
        (
            PublishError::UnchangedCountInconsistent {
                path: "a.rs".into(),
            },
            "unchanged_count_inconsistent",
            Some("a.rs"),
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
    ])
}

#[test]
fn every_publish_failure_maps_to_its_kind_and_path() -> TestResult {
    let mut cases = discovery_log_field_cases();
    cases.extend(publish_log_field_cases()?);
    for (error, kind, path) in &cases {
        ensure_eq(
            &failure_fields(error),
            &(*kind, path.map(Path::new)),
            format!("log fields of {error:?}"),
        )?;
    }
    Ok(())
}
