use super::*;
use crate::sync;
use crate::tools::MAX_PATH_BYTES;
use crate::tools::test_support::activated_project;
use rmcp::handler::server::wrapper::Parameters;
use rusqlite::Connection;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq, ensure_ne};
use std::fs;

fn base_request(project: &tempfile::TempDir, file: &str) -> FindingRequest {
    FindingRequest {
        path: project.path().to_string_lossy().into_owned(),
        file: file.to_owned(),
        line_start: 1,
        line_end: 1,
        severity: "medium".to_owned(),
        category: "correctness".to_owned(),
        title: "Reviewed conclusion".to_owned(),
        description: "A conclusion the AI actually checked.".to_owned(),
    }
}

/// Every finding of this session as `finding_read` returns it.
fn read_all(
    project: &tempfile::TempDir,
    manager: &sync::SourceSyncManager,
) -> TestResult<Vec<crate::tools::FindingReadEntry>> {
    Ok(crate::tools::finding_read(
        &Parameters(crate::tools::FindingReadRequest {
            path: Some(project.path().to_string_lossy().into_owned()),
            file: None,
            cursor: None,
            legacy_paging: crate::tools::LegacyPaging::default(),
        }),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("finding_read succeeds")?
    .0
    .findings)
}

fn status_of(entries: &[crate::tools::FindingReadEntry], id: i64) -> TestResult<String> {
    let found = entries
        .iter()
        .find(|entry| entry.id == id)
        .check(&format!("finding {id} is read back"))?;
    Ok(found.status.clone())
}

fn write_finding_with(
    manager: &sync::SourceSyncManager,
    request: FindingRequest,
) -> TestResult<crate::tools::FindingResponse> {
    Ok(finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        manager,
    )
    .check("finding succeeds")?
    .0)
}

#[test]
fn persists_exactly_the_supplied_conclusion() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let response = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("finding succeeds")?;

    ensure_eq(
        &response.0.status,
        &"current",
        "response.0.status == \"current\"",
    )?;
    ensure(
        !response.0.source_hash.is_empty(),
        "!response.0.source_hash.is_empty()",
    )?;
    ensure(response.0.id > 0, "response.0.id > 0")?;
    Ok(())
}

#[test]
fn a_modified_file_reads_its_finding_as_stale() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = sync::SourceSyncManager::default();
    let id = write_finding_with(&manager, base_request(&project, "lib.rs"))?.id;
    ensure_eq(
        &status_of(&read_all(&project, &manager)?, id)?,
        &"current",
        "status_of(&read_all(&project, &manager)?, id)? == \"current\"",
    )?;

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .check("modify file")?;

    ensure_eq(
        &status_of(&read_all(&project, &manager)?, id)?,
        &"stale",
        "status_of(&read_all(&project, &manager)?, id)? == \"stale\"",
    )?;
    Ok(())
}

#[test]
fn a_deleted_file_reads_its_finding_as_stale() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = sync::SourceSyncManager::default();
    let id = write_finding_with(&manager, base_request(&project, "lib.rs"))?.id;

    fs::remove_file(project.path().join("lib.rs")).check("delete file")?;

    ensure_eq(
        &status_of(&read_all(&project, &manager)?, id)?,
        &"stale",
        "status_of(&read_all(&project, &manager)?, id)? == \"stale\"",
    )?;
    Ok(())
}

#[test]
fn an_untouched_file_keeps_its_finding_current() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    fs::write(project.path().join("other.rs"), b"pub fn b() {}\n").check("write second file")?;
    let manager = sync::SourceSyncManager::default();
    let id = write_finding_with(&manager, base_request(&project, "lib.rs"))?.id;

    fs::write(
        project.path().join("other.rs"),
        b"pub fn b() { changed(); }\n",
    )
    .check("modify other file")?;

    ensure_eq(
        &status_of(&read_all(&project, &manager)?, id)?,
        &"current",
        "status_of(&read_all(&project, &manager)?, id)? == \"current\"",
    )?;
    Ok(())
}

#[test]
fn sync_never_creates_a_finding_on_its_own() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let manager = sync::SourceSyncManager::default();

    ensure(
        read_all(&project, &manager)?.is_empty(),
        "read_all(&project, &manager)?.is_empty()",
    )?;
    ensure(
        !crate::tools::test_support::own_findings_path(&project)?.exists(),
        "!crate::tools::test_support::own_findings_path(&project)?.exists()",
    )?;
    Ok(())
}

#[test]
fn empty_title_is_a_typed_error() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = base_request(&project, "lib.rs");
    request.title = String::new();
    let Err(err) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        return Err("empty title must error".to_owned()).check("unexpected outcome");
    };
    ensure(
        err.message.contains("title"),
        "err.message.contains(\"title\")",
    )?;
    Ok(())
}

#[test]
fn reversed_line_range_is_a_typed_error() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 10;
    request.line_end = 1;
    let Err(err) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        return Err("reversed line range must error".to_owned()).check("unexpected outcome");
    };
    ensure(
        err.message.contains("line_end"),
        "err.message.contains(\"line_end\")",
    )?;
    Ok(())
}

#[test]
fn a_finding_against_an_unindexed_file_is_a_typed_error() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let request = base_request(&project, "does_not_exist.rs");
    let Err(err) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        return Err("unindexed file must error".to_owned()).check("unexpected outcome");
    };
    ensure_eq(
        &err.message,
        &"does_not_exist.rs is not indexed in the current revision",
        "err.message == \"does_not_exist.rs is not indexed in the current revision\"",
    )?;
    let data = err.data.check("an unindexed file carries a payload")?;
    ensure_eq(
        data.get("code").check("data[\"code\"]")?,
        &"file_not_indexed",
        "data[\"code\"] == \"file_not_indexed\"",
    )?;
    Ok(())
}

#[test]
fn a_line_number_past_the_files_real_length_is_a_typed_error() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 1;
    request.line_end = 9_999;
    let Err(err) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        return Err("line number past length must error".to_owned()).check("unexpected outcome");
    };
    ensure(
        err.message.contains("exceeds file line count") || err.message.contains("line_end"),
        "err.message.contains(\"exceeds file line count\") || err.message.contains(\"line_end\")",
    )?;
    Ok(())
}

#[test]
fn a_line_number_within_the_files_real_length_succeeds() -> TestResult {
    let project = activated_project("lib.rs", b"line one\nline two\nline three\n")?;
    let mut request = base_request(&project, "lib.rs");
    request.line_start = 2;
    request.line_end = 3;
    finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    )
    .check("the finding is accepted")?;
    Ok(())
}

#[test]
fn an_oversized_description_is_a_typed_error() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    let mut request = base_request(&project, "lib.rs");
    request.description = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
    let Err(err) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        return Err("oversized description must error".to_owned()).check("unexpected outcome");
    };
    ensure(
        err.message.contains("description") || err.message.contains("character"),
        "err.message.contains(\"description\") || err.message.contains(\"character\")",
    )?;
    Ok(())
}
#[test]
fn an_empty_file_cannot_have_a_finding() -> TestResult {
    // The fixture name deliberately avoids the words the message checks
    // for, and the assertion is the exact message, so neither the file name
    // nor a fixture path can satisfy it on its own.
    let project = activated_project("blank.rs", b"")?;
    let request = base_request(&project, "blank.rs");
    let Err(err) = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    ) else {
        return Err("empty file finding must error".to_owned()).check("unexpected outcome");
    };
    ensure_eq(
        &err.message,
        &"blank.rs has no lines to attach a finding to",
        "err.message == \"blank.rs has no lines to attach a finding to\"",
    )?;
    Ok(())
}

/// The session-scoped dedupe's core contract. Two calls that agree on the
/// conclusion's identity — file, content hash, line range, category, title —
/// must leave **one** row: the second call refreshes the existing row with the
/// latest body (severity, description) and returns the same `id`, rather than
/// appending a near-duplicate.
#[test]
fn restating_a_conclusion_refreshes_one_row_with_the_latest_body() -> TestResult {
    let project = activated_project("lib.rs", b"line one\nline two\n")?;

    let manager = crate::sync::SourceSyncManager::default();
    let first = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("first finding succeeds")?;

    let mut restated = base_request(&project, "lib.rs");
    restated.severity = "critical".to_owned();
    restated.description = "A sharper reading of the same conclusion.".to_owned();
    let second = finding(
        &Parameters(restated),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("restated finding succeeds")?;

    ensure_eq(&second.0.id, &first.0.id, "second.0.id == first.0.id")?;

    let entries = read_all(&project, &manager)?;
    ensure_eq(&entries.len(), &1, "entries.len() == 1")?;
    let entry = entries.first().check("entries[0]")?;
    ensure_eq(&entry.id, &first.0.id, "entry.id == first.0.id")?;
    ensure_eq(
        &entry.severity,
        &"critical",
        "entry.severity == \"critical\"",
    )?;
    ensure_eq(
        &entry.description,
        &"A sharper reading of the same conclusion.",
        "entry.description == \"A sharper reading of the same conclusion.\"",
    )?;
    ensure_eq(
        &entry.title,
        &"Reviewed conclusion",
        "entry.title == \"Reviewed conclusion\"",
    )?;
    Ok(())
}

/// A different target is a different conclusion. Two calls on the same file
/// that name different line ranges must leave two rows, each keeping its own
/// id — dedupe must not collapse distinct subjects onto one row.
#[test]
fn distinct_targets_leave_distinct_rows() -> TestResult {
    let project = activated_project("lib.rs", b"line one\nline two\n")?;
    let manager = crate::sync::SourceSyncManager::default();
    let first = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("first finding succeeds")?;

    let mut elsewhere = base_request(&project, "lib.rs");
    elsewhere.line_start = 2;
    elsewhere.line_end = 2;
    let second = finding(
        &Parameters(elsewhere),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("second finding succeeds")?;

    ensure_ne(&second.0.id, &first.0.id, "second.0.id != first.0.id")?;
    ensure_eq(
        &read_all(&project, &manager)?.len(),
        &2,
        "read_all(&project, &manager)?.len() == 2",
    )?;
    Ok(())
}

/// The content hash is part of the identity: a conclusion restated after its
/// file changed rests on different evidence, so the old row goes stale and the
/// restatement is a new row. Dedupe must not silently revive a stale row.
#[test]
fn a_restatement_after_the_file_changed_is_a_new_row() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;

    let manager = crate::sync::SourceSyncManager::default();
    let first = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("first finding succeeds")?;

    fs::write(
        project.path().join("lib.rs"),
        b"pub fn a() { changed(); }\n",
    )
    .check("modify file")?;
    let second = finding(
        &Parameters(base_request(&project, "lib.rs")),
        &crate::progress::NoopProgressSink,
        &manager,
    )
    .check("restated finding succeeds")?;

    ensure_ne(&second.0.id, &first.0.id, "second.0.id != first.0.id")?;
    let entries = read_all(&project, &manager)?;
    ensure_eq(&entries.len(), &2, "entries.len() == 2")?;
    ensure_eq(
        &status_of(&entries, first.0.id)?,
        &"stale",
        "status_of(&entries, first.0.id)? == \"stale\"",
    )?;
    ensure_eq(
        &status_of(&entries, second.0.id)?,
        &"current",
        "status_of(&entries, second.0.id)? == \"current\"",
    )?;
    Ok(())
}

#[test]
fn a_finding_against_a_binary_file_carries_the_file_not_indexed_code() -> TestResult {
    let project = activated_project("lib.rs", b"pub fn a() {}\n")?;
    fs::write(project.path().join("blob.dat"), b"\0\x01binary\n").check("binary file")?;
    let request = base_request(&project, "blob.dat");

    let result = finding(
        &Parameters(request),
        &crate::progress::NoopProgressSink,
        &crate::sync::SourceSyncManager::default(),
    );

    let Err(error) = result else {
        return Err("a binary file has no indexed content".to_owned()).check("unexpected outcome");
    };
    let data = error.data.check("a binary file carries a payload")?;
    ensure_eq(
        data.get("code").check("data[\"code\"]")?,
        &"file_not_indexed",
        format!("{}", error.message),
    )?;
    Ok(())
}

#[path = "finding_failure_tests.rs"]
mod finding_failure_tests;
