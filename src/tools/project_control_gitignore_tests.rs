//! Tests for the `.gitignore` rule that `project_control on` adds, and off leaves alone.
use super::*;

#[test]
fn on_action_reports_the_gitignore_rule_it_added() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable succeeds")?;

    let report = response
        .0
        .gitignore
        .check("enable reports the gitignore guard")?;
    let expected = project
        .path()
        .canonicalize()
        .check("tempdir canonicalizes")?
        .join(".gitignore");
    ensure_eq(
        &report.status,
        &GitignoreStatus::Added,
        "report.status == GitignoreStatus::Added",
    )?;
    ensure_eq(
        &report.path,
        &expected.to_string_lossy(),
        "report.path == expected.to_string_lossy()",
    )?;
    ensure(
        report.error.is_none(),
        "a successful append carries no error",
    )?;
    ensure(
        fs::read_to_string(&expected)
            .check("read .gitignore")?
            .contains(".planning/slugaudit/"),
        "the rule is on disk",
    )?;
    Ok(())
}

#[test]
fn on_action_reports_an_already_ignored_project_without_duplicating_the_rule() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let gitignore = project.path().join(".gitignore");
    let original = b"/target\n/.planning/slugaudit/\n";
    fs::write(&gitignore, original).check("write .gitignore")?;

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable succeeds")?;

    let report = response
        .0
        .gitignore
        .check("enable reports the gitignore guard")?;
    ensure_eq(
        &report.status,
        &GitignoreStatus::AlreadyIgnored,
        "report.status == GitignoreStatus::AlreadyIgnored",
    )?;
    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::Enabled,
        "response.0.status == ProjectControlStatus::Enabled",
    )?;
    response.0.import.as_ref().check("the import still ran")?;
    ensure_eq(
        &fs::read(&gitignore).check("read .gitignore")?,
        &original.to_vec(),
        "an already-ignored project's file stays byte-identical",
    )?;
    Ok(())
}

#[test]
fn on_action_enables_the_project_even_when_the_gitignore_cannot_be_written() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    fs::create_dir(project.path().join(".gitignore")).check("block the ignore file")?;

    let response = project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("the convenience guard must not fail the enable")?;

    let report = response
        .0
        .gitignore
        .check("enable reports the gitignore guard")?;
    ensure_eq(
        &report.status,
        &GitignoreStatus::NotWritten,
        "report.status == GitignoreStatus::NotWritten",
    )?;
    report.error.as_ref().check("the reason is reported")?;
    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::Enabled,
        "response.0.status == ProjectControlStatus::Enabled",
    )?;
    ensure(
        activation_dir(&project).join("project.db").exists(),
        "the project is enabled regardless of the ignore guard",
    )?;
    Ok(())
}

#[test]
fn off_action_does_not_touch_the_gitignore() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    project_control(&on_request(&path), &crate::progress::NoopProgressSink, None)
        .check("enable")?;
    let gitignore = project.path().join(".gitignore");
    let after_enable = fs::read(&gitignore).check("read .gitignore")?;

    let response = project_control(
        &off_request(&path),
        &crate::progress::NoopProgressSink,
        None,
    )
    .check("disable succeeds")?;

    ensure_eq(
        &response.0.status,
        &ProjectControlStatus::Disabled,
        "response.0.status == ProjectControlStatus::Disabled",
    )?;
    ensure(
        response.0.gitignore.is_none(),
        "off omits the gitignore field",
    )?;
    ensure_eq(
        &fs::read(&gitignore).check("read .gitignore")?,
        &after_enable,
        "off leaves the user's ignore file alone",
    )?;
    Ok(())
}

#[test]
fn an_unknown_profile_is_refused_before_anything_is_enabled() -> TestResult {
    let (project, path) = temp_project_with_source()?;
    let req = Parameters(ProjectControlRequest {
        path: Some(path),
        action: ProjectControlAction::On,
        profile: Some("strict".to_string()),
    });

    let error = project_control(&req, &crate::progress::NoopProgressSink, None)
        .err()
        .check("a profile that does not exist must not enable on the default")?;

    ensure(
        error.message.contains("unknown profile `strict`"),
        format!("{}", error.message),
    )?;
    ensure(
        !activation_dir(&project).exists(),
        "the refused call must leave the project disabled",
    )?;
    Ok(())
}
