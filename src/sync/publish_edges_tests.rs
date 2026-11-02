//! End-to-end coverage that dependency edges actually land in the database
//! through the real publish pipeline (discovery → sample → parse → the
//! graph resolver → write), not just at the unit level inside
//! `graph::resolve_imports` itself.
use super::*;
use crate::store::open_read_write;
use crate::sync::test_support::write;
use slugaudit_test_check::{Check, TestResult};
use std::fs;

#[derive(Debug)]
struct Edge {
    from: String,
    to: Option<String>,
    kind: String,
}

fn edges(connection: &Connection) -> TestResult<Vec<Edge>> {
    let mut statement = connection
        .prepare(
            "SELECT f.path, t.path, e.resolution_kind FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             LEFT JOIN files t ON t.id = e.to_file_id \
             ORDER BY f.path, e.raw_import_text",
        )
        .check("prepare")?;
    statement
        .query_map([], |row| {
            Ok(Edge {
                from: row.get(0)?,
                to: row.get(1)?,
                kind: row.get(2)?,
            })
        })
        .check("query")?
        .collect::<Result<_, _>>()
        .check("collect")
}

#[test]
fn a_python_relative_import_resolves_to_the_real_sibling_file() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "pkg/__init__.py", b"")?;
    write(project.path(), "pkg/helper.py", b"def f():\n    pass\n")?;
    write(project.path(), "pkg/main.py", b"from .helper import f\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("publish")?;

    let all = edges(&connection)?;
    let resolved = all
        .iter()
        .find(|edge| edge.from == "pkg/main.py" && edge.kind == "Resolved")
        .check(&format!(
            "a resolved edge from pkg/main.py among {:?}",
            all.iter()
                .map(|e| (&e.from, &e.to, &e.kind))
                .collect::<Vec<_>>()
        ))?;
    assert_eq!(resolved.to.as_deref(), Some("pkg/helper.py"));
    Ok(())
}

#[test]
fn a_rust_crate_use_path_resolves_to_the_real_module_file() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(
        project.path(),
        "src/main.rs",
        b"use crate::helper::greet;\nfn main() {}\n",
    )?;
    write(project.path(), "src/helper.rs", b"pub fn greet() {}\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("publish")?;

    let all = edges(&connection)?;
    let resolved = all
        .iter()
        .find(|edge| edge.from == "src/main.rs" && edge.kind == "Resolved")
        .check("a resolved edge")?;
    assert_eq!(resolved.to.as_deref(), Some("src/helper.rs"));
    Ok(())
}

#[test]
fn an_external_crate_use_is_recorded_as_external_with_no_target() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(
        project.path(),
        "src/main.rs",
        b"use std::collections::HashMap;\nfn main() {}\n",
    )?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("publish")?;

    let all = edges(&connection)?;
    let external = all
        .iter()
        .find(|edge| edge.from == "src/main.rs")
        .check("an edge for main.rs")?;
    assert_eq!(external.kind, "External");
    assert_eq!(external.to, None);
    Ok(())
}

#[test]
fn a_modified_file_gets_its_edges_replaced_not_duplicated() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "pkg/__init__.py", b"")?;
    write(project.path(), "pkg/a.py", b"")?;
    write(project.path(), "pkg/b.py", b"")?;
    write(project.path(), "pkg/main.py", b"from .a import x\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("first publish")?;

    write(project.path(), "pkg/main.py", b"from .b import y\n")?;
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("second publish")?;

    let all = edges(&connection)?;
    let from_main: Vec<&Edge> = all
        .iter()
        .filter(|edge| edge.from == "pkg/main.py")
        .collect();
    assert_eq!(
        from_main.len(),
        1,
        "the stale edge to a.py must be gone, not accumulated alongside the new one"
    );
    assert_eq!(from_main[0].to.as_deref(), Some("pkg/b.py"));
    Ok(())
}

#[test]
fn generic_walker_languages_get_edges_through_the_full_pipeline() -> TestResult {
    // Languages the pack's own import pass doesn't cover (swift, csharp,
    // dart, c, julia) must still produce dependency edges via the generic
    // import walker, resolved against real project files.
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "App.swift", b"import Foundation\n")?;
    write(project.path(), "Program.cs", b"using Core.Helper;\n")?;
    write(
        project.path(),
        "Core/Helper.cs",
        b"namespace Core { class Helper {} }\n",
    )?;
    write(
        project.path(),
        "lib/main.dart",
        b"import '../helper.dart';\nvoid main() {}\n",
    )?;
    write(project.path(), "helper.dart", b"int helper() => 1;\n")?;
    write(project.path(), "main.c", b"#include \"local.h\"\n")?;
    write(project.path(), "local.h", b"#define X 1\n")?;
    write(project.path(), "main.jl", b"using .Helper\n")?;
    write(project.path(), "Helper.jl", b"module Helper end\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;

    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("publish")?;

    let all = edges(&connection)?;
    let assert_edge = |from: &str, to: &str| {
        assert!(
            all.iter().any(|edge| edge.from == from
                && edge.kind == "Resolved"
                && edge.to.as_deref() == Some(to)),
            "expected a Resolved edge {from} -> {to} among {all:?}"
        );
    };
    assert_edge("lib/main.dart", "helper.dart");
    assert_edge("main.c", "local.h");
    assert_edge("Program.cs", "Core/Helper.cs");
    assert_edge("main.jl", "Helper.jl");

    // swift's module import is honestly External (no project file), not
    // faked as resolved.
    let swift_edge = all
        .iter()
        .find(|edge| edge.from == "App.swift")
        .check("an edge for App.swift")?;
    assert_eq!(swift_edge.kind, "External");
    assert_eq!(swift_edge.to, None);
    Ok(())
}

#[test]
fn a_deleted_target_file_cascades_away_its_incoming_edge() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "pkg/__init__.py", b"")?;
    write(project.path(), "pkg/helper.py", b"")?;
    write(project.path(), "pkg/main.py", b"from .helper import f\n")?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("first publish")?;
    assert!(
        edges(&connection)?
            .iter()
            .any(|edge| edge.kind == "Resolved")
    );

    fs::remove_file(project.path().join("pkg/helper.py")).check("delete target")?;
    publish(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
    )
    .check("second publish")?;

    assert!(
        !edges(&connection)?
            .iter()
            .any(|edge| edge.to.as_deref() == Some("pkg/helper.py")),
        "an edge pointing at a deleted file must not survive"
    );
    Ok(())
}

/// An import whose target is discovered on disk but skipped by sampling
/// (here: oversize) must still resolve. Reporting it `Unresolved` would
/// read as *dead code* to an auditing agent, which is a fabricated finding
/// — the exact lie `known_paths` exists to prevent. The target also has
/// a `skipped` existence row, so the edge resolves to a real `files.id`
/// rather than only being classified `Resolved`; what this test pins is the
/// verdict either way.
#[test]
fn an_import_to_a_discovered_but_oversize_skipped_file_resolves() -> TestResult {
    let project = tempfile::tempdir().check("project dir")?;
    write(project.path(), "pkg/__init__.py", b"")?;
    write(project.path(), "pkg/main.py", b"from .helper import f\n")?;
    // Larger than `max_file_bytes` below, so sampling skips it while
    // discovery still walks it.
    write(project.path(), "pkg/helper.py", &[b'#'; 200])?;
    let db_dir = tempfile::tempdir().check("db dir")?;
    let mut connection = open_read_write(&db_dir.path().join("project.db")).check("open db")?;
    let limits = ResourceLimits {
        max_file_bytes: 50,
        max_total_import_bytes: 100_000,
        ..ResourceLimits::default()
    };

    let report = publish_with_limits(
        &mut connection,
        project.path(),
        "1.0.0",
        &crate::progress::NoopProgressSink,
        &limits,
    )
    .check("publish")?;

    assert!(
        report
            .skipped
            .iter()
            .any(|file| file.absolute_path.ends_with("pkg/helper.py")),
        "helper.py must be skipped as oversize, got {:?}",
        report.skipped
    );

    let all = edges(&connection)?;
    let imported = all
        .iter()
        .find(|edge| edge.from == "pkg/main.py")
        .check(&format!("an edge from pkg/main.py among {all:?}"))?;
    assert_eq!(
        imported.kind, "Resolved",
        "an import of a discovered-but-skipped file must not be reported Unresolved: {all:?}"
    );
    assert_eq!(
        imported.to.as_deref(),
        Some("pkg/helper.py"),
        "the skipped file's existence row is the edge's resolved target: {all:?}"
    );
    Ok(())
}
