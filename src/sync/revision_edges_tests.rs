use super::*;
use crate::model::{
    EvidenceItem, EvidenceOrigin, ExtractionCompleteness, ParseOutcome, ParserAvailability,
    ParserRun, SourceIdentity, SpanAvailability,
};
use crate::store::open_read_write;
use crate::sync::revision::{RevisionInputs, publish_revision};
use rusqlite::Connection;
use serde_json::json;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

fn rust_record(path: &str, hash: &str, evidence: Vec<EvidenceItem>) -> FileRecord {
    FileRecord {
        relative_path: path.to_owned(),
        is_binary: false,
        content: Some(String::new()),
        identity: SourceIdentity {
            relative_path: path.to_owned(),
            content_hash: hash.to_owned(),
            hash_algorithm: "blake3".to_owned(),
        },
        byte_len: 0,
        modified_unix_seconds: None,
        language: Some("rust".to_owned()),
        language_detected: true,
        run: ParserRun {
            availability: ParserAvailability::Available,
            outcome: ParseOutcome::Succeeded,
            completeness: ExtractionCompleteness::Full,
        },
        evidence,
    }
}

fn import_evidence(index: usize, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("import:{index}"),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: json!({
            "source": source,
            "items": [],
            "alias": null,
            "is_wildcard": source.contains('*'),
        }),
    }
}

fn path_mod_evidence(index: usize, name: &str, path_attr: &str, inline_dir: &str) -> EvidenceItem {
    let inline_depth = inline_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count();
    EvidenceItem {
        key: format!("structure:{index}"),
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: json!({
            "kind": "Module",
            "name": name,
            "rust_mod_path": {
                "path_attr": path_attr,
                "inline_dir": inline_dir,
                "inline_depth": inline_depth,
            }
        }),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct StoredEdge {
    from: String,
    to: Option<String>,
    kind: String,
    syntax_unmodeled: bool,
}

fn query_edges(connection: &Connection) -> TestResult<Vec<StoredEdge>> {
    let mut stmt = connection
        .prepare(
            "SELECT f.path, t.path, e.resolution_kind, e.syntax_unmodeled \
             FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             LEFT JOIN files t ON t.id = e.to_file_id \
             ORDER BY f.path, COALESCE(t.path, ''), e.resolution_kind",
        )
        .check("prepare edge query")?;
    stmt.query_map([], |row| {
        Ok(StoredEdge {
            from: row.get(0)?,
            to: row.get(1)?,
            kind: row.get(2)?,
            syntax_unmodeled: row.get(3)?,
        })
    })
    .check("run edge query")?
    .collect::<Result<_, _>>()
    .check("collect stored edges")
}

fn publish_step(
    connection: &mut Connection,
    expected_current: Option<&str>,
    manifest: &str,
    upserts: &[FileRecord],
    deletions: &[String],
    known: &[&str],
    re_resolve: bool,
) -> TestResult<String> {
    let discovered: HashSet<String> = known.iter().map(|path| (*path).to_owned()).collect();
    publish_revision(
        connection,
        &RevisionInputs {
            expected_current,
            manifest_hash: manifest,
            parser_pack_version: "1.0.0",
            upserts,
            deletions,
            skipped: &[],
            discovered_paths: &discovered,
            re_resolve_unresolved: re_resolve,
        },
    )
    .check("publish revision")
}

#[test]
fn grouped_use_resolves_incrementally_across_revisions_without_pop_loss() -> TestResult {
    let db_dir = tempfile::tempdir().check("create temporary db directory")?;
    let mut conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;

    let caller = rust_record(
        "src/caller.rs",
        "h1",
        vec![
            import_evidence(0, "use crate::{a, b};"),
            import_evidence(1, "use crate::foo::{};"),
            import_evidence(2, "pubfn use broken;"),
        ],
    );
    let a = rust_record("src/a.rs", "h2", vec![]);
    let rev1 = publish_step(
        &mut conn,
        None,
        "m1",
        &[caller, a],
        &[],
        &["src/caller.rs", "src/a.rs"],
        false,
    )?;

    ensure_eq(
        &query_edges(&conn)?,
        &vec![
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: None,
                kind: "Unresolved".to_owned(),
                syntax_unmodeled: false,
            },
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: None,
                kind: "Unresolved".to_owned(),
                syntax_unmodeled: true,
            },
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/a.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
        ],
        "initial publish emits resolved a.rs, unresolved b, and unmodeled broken edge",
    )?;

    let b = rust_record("src/b.rs", "h3", vec![]);
    publish_step(
        &mut conn,
        Some(&rev1),
        "m2",
        &[b],
        &[],
        &["src/caller.rs", "src/a.rs", "src/b.rs"],
        true,
    )?;

    ensure_eq(
        &query_edges(&conn)?,
        &vec![
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: None,
                kind: "Unresolved".to_owned(),
                syntax_unmodeled: true,
            },
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/a.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/b.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
        ],
        "incremental publish re-resolves unchanged caller.rs to both a.rs and b.rs",
    )?;
    Ok(())
}

#[test]
fn adding_rust_module_upgrades_low_confidence_crate_root_edge() -> TestResult {
    let db_dir = tempfile::tempdir().check("create temporary db directory")?;
    let mut conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;

    let lib = rust_record("src/lib.rs", "hl1", vec![]);
    let caller = rust_record(
        "src/caller.rs",
        "hc1",
        vec![import_evidence(0, "use crate::{a, b};")],
    );
    let a = rust_record("src/a.rs", "ha1", vec![]);
    let rev1 = publish_step(
        &mut conn,
        None,
        "m1",
        &[lib, caller, a],
        &[],
        &["src/lib.rs", "src/caller.rs", "src/a.rs"],
        false,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/a.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/lib.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
        ],
        "while b.rs is absent, crate::b falls back to the crate root with Low confidence",
    )?;

    let b = rust_record("src/b.rs", "hb1", vec![]);
    publish_step(
        &mut conn,
        Some(&rev1),
        "m2",
        &[b],
        &[],
        &["src/lib.rs", "src/caller.rs", "src/a.rs", "src/b.rs"],
        true,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/a.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
            StoredEdge {
                from: "src/caller.rs".to_owned(),
                to: Some("src/b.rs".to_owned()),
                kind: "Resolved".to_owned(),
                syntax_unmodeled: false,
            },
        ],
        "adding b.rs upgrades unchanged caller.rs edge from lib.rs to b.rs",
    )?;
    Ok(())
}

#[test]
fn path_attr_super_edges_update_when_host_added_or_modified() -> TestResult {
    let db_dir = tempfile::tempdir().check("create temporary db directory")?;
    let mut conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;

    let tests = rust_record(
        "src/foo_tests.rs",
        "ht1",
        vec![import_evidence(0, "use super::*;")],
    );
    let lib = rust_record("src/lib.rs", "hl1", vec![]);
    let rev1 = publish_step(
        &mut conn,
        None,
        "m1",
        &[tests, lib],
        &[],
        &["src/foo_tests.rs", "src/lib.rs"],
        false,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![StoredEdge {
            from: "src/foo_tests.rs".to_owned(),
            to: Some("src/lib.rs".to_owned()),
            kind: "Resolved".to_owned(),
            syntax_unmodeled: false,
        }],
        "without a declaring file, super::* falls back to the crate root",
    )?;

    let foo_plain = rust_record("src/foo.rs", "hf1", vec![]);
    let rev2 = publish_step(
        &mut conn,
        Some(&rev1),
        "m2",
        &[foo_plain],
        &[],
        &["src/foo_tests.rs", "src/lib.rs", "src/foo.rs"],
        true,
    )?;

    let foo_with_path = rust_record(
        "src/foo.rs",
        "hf2",
        vec![path_mod_evidence(0, "tests", "foo_tests.rs", "")],
    );
    publish_step(
        &mut conn,
        Some(&rev2),
        "m3",
        &[foo_with_path],
        &[],
        &["src/foo_tests.rs", "src/lib.rs", "src/foo.rs"],
        false,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![StoredEdge {
            from: "src/foo_tests.rs".to_owned(),
            to: Some("src/foo.rs".to_owned()),
            kind: "Resolved".to_owned(),
            syntax_unmodeled: false,
        }],
        "modifying foo.rs to declare #[path] re-resolves unchanged foo_tests.rs to foo.rs",
    )?;
    Ok(())
}

#[test]
fn path_attr_super_edges_revert_when_host_drops_attr_or_is_deleted() -> TestResult {
    let db_dir = tempfile::tempdir().check("create temporary db directory")?;
    let mut conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;

    let tests = rust_record(
        "src/foo_tests.rs",
        "ht1",
        vec![import_evidence(0, "use super::*;")],
    );
    let lib = rust_record("src/lib.rs", "hl1", vec![]);
    let foo_with_path = rust_record(
        "src/foo.rs",
        "hf1",
        vec![path_mod_evidence(0, "tests", "foo_tests.rs", "")],
    );
    let rev1 = publish_step(
        &mut conn,
        None,
        "m1",
        &[tests, lib, foo_with_path],
        &[],
        &["src/foo_tests.rs", "src/lib.rs", "src/foo.rs"],
        false,
    )?;

    let foo_removed = rust_record("src/foo.rs", "hf2", vec![]);
    let rev2 = publish_step(
        &mut conn,
        Some(&rev1),
        "m2",
        &[foo_removed],
        &[],
        &["src/foo_tests.rs", "src/lib.rs", "src/foo.rs"],
        false,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![StoredEdge {
            from: "src/foo_tests.rs".to_owned(),
            to: Some("src/lib.rs".to_owned()),
            kind: "Resolved".to_owned(),
            syntax_unmodeled: false,
        }],
        "removing #[path] from foo.rs re-resolves unchanged foo_tests.rs back to lib.rs",
    )?;

    let foo_again = rust_record(
        "src/foo.rs",
        "hf3",
        vec![path_mod_evidence(0, "tests", "foo_tests.rs", "")],
    );
    let rev3 = publish_step(
        &mut conn,
        Some(&rev2),
        "m3",
        &[foo_again],
        &[],
        &["src/foo_tests.rs", "src/lib.rs", "src/foo.rs"],
        false,
    )?;
    publish_step(
        &mut conn,
        Some(&rev3),
        "m4",
        &[],
        &["src/foo.rs".to_owned(), "src/lib.rs".to_owned()],
        &["src/foo_tests.rs"],
        true,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![StoredEdge {
            from: "src/foo_tests.rs".to_owned(),
            to: None,
            kind: "Unresolved".to_owned(),
            syntax_unmodeled: false,
        }],
        "deleting the declaring file reverts the child edge to Unresolved",
    )?;
    Ok(())
}

#[test]
fn removing_path_attr_updates_child_super_super_edge_pointing_elsewhere() -> TestResult {
    let db_dir = tempfile::tempdir().check("create temporary db directory")?;
    let mut conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;

    let parent_with_path = rust_record(
        "src/a/parent.rs",
        "hp1",
        vec![path_mod_evidence(0, "tests", "../child.rs", "")],
    );
    let child = rust_record(
        "src/child.rs",
        "hc1",
        vec![import_evidence(0, "use super::super::target::Item;")],
    );
    let target_under_a = rust_record("src/a/target.rs", "ht1", vec![]);
    let rev1 = publish_step(
        &mut conn,
        None,
        "m1",
        &[parent_with_path, child, target_under_a],
        &[],
        &["src/a/parent.rs", "src/child.rs", "src/a/target.rs"],
        false,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![StoredEdge {
            from: "src/child.rs".to_owned(),
            to: Some("src/a/target.rs".to_owned()),
            kind: "Resolved".to_owned(),
            syntax_unmodeled: false,
        }],
        "while parent.rs declares child.rs via #[path], super::super::target resolves to src/a/target.rs",
    )?;

    let parent_without_path = rust_record("src/a/parent.rs", "hp2", vec![]);
    publish_step(
        &mut conn,
        Some(&rev1),
        "m2",
        &[parent_without_path],
        &[],
        &["src/a/parent.rs", "src/child.rs", "src/a/target.rs"],
        false,
    )?;
    ensure_eq(
        &query_edges(&conn)?,
        &vec![StoredEdge {
            from: "src/child.rs".to_owned(),
            to: None,
            kind: "Unresolved".to_owned(),
            syntax_unmodeled: false,
        }],
        "when parent.rs drops #[path] while child.rs is unchanged, stale super::super edge to target.rs is cleared",
    )?;
    Ok(())
}
