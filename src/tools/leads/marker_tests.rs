//! `leads` over the `comment_markers` mechanic: detail clipping and the
//! whole-word, upper-case match rule.
use super::tests::{activate, full_rows, request, run};
use super::*;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::fs;

#[test]
fn comment_marker_detail_is_clipped_at_the_char_limit() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    // Multi-byte padding proves the cut lands on a char boundary, not a byte
    // offset: 300 two-byte chars, truncated to the cap.
    let short = "y".repeat(120);
    let long = format!("// TODO: {}\n", "é".repeat(300));
    fs::write(project.path().join("long.rs"), long).check("write long")?;
    let body = format!("// TODO: {short}\n");
    fs::write(project.path().join("short.rs"), body).check("write short")?;

    let response = run(request(project.path(), Some("comment_markers")))?;
    let rows = full_rows(&response)?;

    ensure_eq(&rows.len(), &2, "rows.len() == 2")?;
    ensure_eq(
        &rows.first().check("rows[0]")?.detail.chars().count(),
        &MAX_DETAIL_CHARS,
        "rows[0].detail.chars().count() == MAX_DETAIL_CHARS",
    )?;
    ensure(
        rows.first().check("rows[0]")?.detail_clipped,
        "rows[0].detail_clipped",
    )?;
    ensure(
        !rows.get(1).check("rows[1]")?.detail_clipped,
        "!rows[1].detail_clipped",
    )?;
    ensure(
        rows.get(1).check("rows[1]")?.detail.ends_with(&short),
        &rows.get(1).check("rows[1]")?.detail,
    )?;
    Ok(())
}

/// A marker counts only as a whole, upper-case word: lower-case text and a
/// marker inside a longer word ("whack", "shackle", "maxxxed") do not.
#[test]
fn comment_markers_match_whole_upper_case_words_only() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    activate(project.path())?;
    fs::write(
        project.path().join("miss.rs"),
        "// whack the shackle\n// todo: lower case\n// maxxxed out\n// HACKS and TODOS\nfn a() {}\n",
    )
    .check("write non-matching comments")?;
    fs::write(
        project.path().join("hit.rs"),
        "// HACK\n// x FIXME(bob)\n// see XXX.\n// TODO: end\nfn b() {}\n",
    )
    .check("write matching comments")?;

    let response = run(request(project.path(), Some("comment_markers")))?;
    let rows = full_rows(&response)?;

    ensure_eq(&rows.len(), &4, "rows.len() == 4")?;
    ensure(
        rows.iter().all(|row| row.file == "hit.rs"),
        "rows.iter().all(|row| row.file == \"hit.rs\")",
    )?;
    Ok(())
}
