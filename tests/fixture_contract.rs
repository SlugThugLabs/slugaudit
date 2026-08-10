//! Phase 12 acceptance contract (Task 12.1): asserts the checked-in
//! multi-language fixture's versioned golden manifest
//! (`tests/fixtures/multilang/MANIFEST.json`) against what a real
//! `sync::publish` produces from a *copy* of the fixture.
//!
//! Why a copy: the test must never mutate the checked-in fixture, and the
//! temp copy keeps the walk hermetic (no parent git repository, so the
//! repo's own `.gitignore` cannot leak into the walk — only the fixture's
//! own ignore rules apply, which is the deterministic contract).
//!
//! Two modes:
//! - Normal (default): asserts the database matches the manifest exactly
//!   (file set, per-file language/status/evidence counts/content hash,
//!   dependency edges, binary classification, searchability).
//! - Regeneration: `SLUGAUDIT_REGEN_MANIFEST=1` rewrites `MANIFEST.json`
//!   from current output. Use only when the parser pack version is
//!   deliberately bumped, then review the regenerated manifest by hand
//!   before committing (golden-manifest discipline — plan-audit PHASE-12).

use rusqlite::Connection;
use serde_json::{Map, Value, json};
use slugaudit_mcp_rust::parse::PACK_VERSION;
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::store::open_read_write;
use slugaudit_mcp_rust::sync::publish;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const FIXTURE_DIR: &str = "tests/fixtures/multilang";
const MANIFEST_FILE: &str = "tests/fixtures/multilang/MANIFEST.json";
/// Bump deliberately when the manifest's meaning changes (new fields,
/// stricter expectations), independent of the parser pack version.
const CONTRACT_VERSION: u64 = 1;
/// Files that are part of the fixture repo but must not be indexed when
/// the contract publishes a copy of it.
const NON_INDEXED_FIXTURE_FILES: &[&str] = &["MANIFEST.json", "CONTRACT.md"];

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR)
}

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(MANIFEST_FILE)
}

fn copy_dir_all(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create destination dir");
    for entry in fs::read_dir(source).expect("read source dir") {
        let entry = entry.expect("read dir entry");
        let name = entry.file_name();
        let name_str = name.to_string_lossy().into_owned();
        if entry.file_type().expect("entry type").is_dir() {
            copy_dir_all(&entry.path(), &destination.join(&name));
        } else if !NON_INDEXED_FIXTURE_FILES.contains(&name_str.as_str()) {
            fs::copy(entry.path(), destination.join(&name)).expect("copy fixture file");
        }
    }
}

/// Publishes a copy of the fixture into a temp dir and returns the open
/// connection plus the temp dir (kept alive for the connection's lifetime).
fn publish_fixture_copy() -> (tempfile::TempDir, Connection) {
    let work = tempfile::tempdir().expect("temp dir for fixture copy");
    copy_dir_all(&fixture_root(), work.path());
    // The database lives in the activation directory, exactly like
    // production: discovery excludes `.planning/slugaudit/`, so the
    // database must never appear in the indexed file set. Placing it at
    // the project root (a bug this test caught on its first run) would
    // index the database files themselves as binary content.
    fs::create_dir_all(work.path().join(".planning/slugaudit")).expect("activation dir");
    let database = work.path().join(".planning/slugaudit/project.db");
    let mut connection = open_read_write(&database).expect("open contract database");
    let report = publish(
        &mut connection,
        work.path(),
        PACK_VERSION,
        &NoopProgressSink,
    )
    .expect("publish the fixture copy");
    assert_eq!(
        report.deleted, 0,
        "a fresh publish of an unchanged fixture must not report deletions"
    );
    (work, connection)
}

// ---- database extraction helpers -----------------------------------------

fn stored_paths(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT path FROM files ORDER BY path")
        .expect("prepare paths");
    statement
        .query_map([], |row| row.get(0))
        .expect("query paths")
        .collect::<Result<_, _>>()
        .expect("collect paths")
}

fn file_record(connection: &Connection, path: &str) -> Value {
    let (file_kind, language, parse_outcome, completeness, byte_len, content_hash): (
        String,
        Option<String>,
        String,
        String,
        i64,
        String,
    ) = connection
        .query_row(
            "SELECT file_kind, language, parse_outcome, extraction_completeness, \
             byte_len, content_hash FROM files WHERE path = ?1",
            [path],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap_or_else(|error| panic!("expected a row for {path}: {error}"));
    let evidence_counts: BTreeMap<String, i64> = {
        let mut statement = connection
            .prepare(
                "SELECT kind, count(*) FROM evidence e JOIN files f ON f.id = e.file_id \
                 WHERE f.path = ?1 GROUP BY kind ORDER BY kind",
            )
            .expect("prepare evidence counts");
        statement
            .query_map([path], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query evidence counts")
            .collect::<Result<_, _>>()
            .expect("collect evidence counts")
    };
    json!({
        "file_kind": file_kind,
        "language": language,
        "parse_outcome": parse_outcome,
        "extraction_completeness": completeness,
        "byte_len": byte_len,
        "content_hash": content_hash,
        "evidence_counts": evidence_counts,
    })
}

fn evidence_by_kind(connection: &Connection) -> BTreeMap<String, i64> {
    let mut statement = connection
        .prepare("SELECT kind, count(*) FROM evidence GROUP BY kind ORDER BY kind")
        .expect("prepare evidence totals");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query evidence totals")
        .collect::<Result<_, _>>()
        .expect("collect evidence totals")
}

fn resolved_edges(connection: &Connection) -> Vec<(String, String)> {
    let mut statement = connection
        .prepare(
            "SELECT f.path, t.path FROM dependency_edges e \
             JOIN files f ON f.id = e.from_file_id \
             JOIN files t ON t.id = e.to_file_id \
             WHERE e.resolution_kind = 'Resolved' ORDER BY f.path, t.path",
        )
        .expect("prepare resolved edges");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query resolved edges")
        .collect::<Result<_, _>>()
        .expect("collect resolved edges")
}

fn edge_count(connection: &Connection, kind: &str) -> i64 {
    connection
        .query_row(
            "SELECT count(*) FROM dependency_edges WHERE resolution_kind = ?1",
            [kind],
            |row| row.get(0),
        )
        .expect("count edges")
}

fn unsupported_language_unresolved_count(connection: &Connection) -> i64 {
    let mut statement = connection
        .prepare(
            "SELECT f.language FROM dependency_edges de \
             JOIN files f ON f.id = de.from_file_id \
             WHERE de.resolution_kind = 'Unresolved'",
        )
        .expect("prepare unresolved languages");
    let languages: Vec<Option<String>> = statement
        .query_map([], |row| row.get(0))
        .expect("query unresolved languages")
        .collect::<Result<_, _>>()
        .expect("collect unresolved languages");
    languages
        .into_iter()
        .flatten()
        .filter(|language| !slugaudit_mcp_rust::graph::is_supported_language(language))
        .count() as i64
}

fn indexed_with_content(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT count(*) FROM files WHERE file_kind = 'indexed' AND content IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("count indexed files with content")
}

/// Builds the manifest document from the current database state.
fn build_manifest(connection: &Connection) -> Value {
    let paths = stored_paths(connection);
    let files: Map<String, Value> = paths
        .iter()
        .map(|path| (path.clone(), file_record(connection, path)))
        .collect();
    let indexed = files
        .values()
        .filter(|record| record["file_kind"] == json!("indexed"))
        .count();
    let binary = paths.len() - indexed;
    let resolved = resolved_edges(connection);
    json!({
        "contract_version": CONTRACT_VERSION,
        "parser_pack_version": PACK_VERSION,
        "notes": "Informational. Reviewed by hand: malformed Python flagged as SyntaxErrors with diagnostics, both circular import pairs resolve in both directions, binary classified inert, Go/Ruby imports honestly Unresolved (unsupported languages). Per-file content_hash pins the fixture source itself, so any fixture edit fails the contract until the manifest is deliberately regenerated.",
        "file_set": paths,
        "files": files,
        "totals": {
            "file_count": paths.len(),
            "indexed": indexed,
            "binary": binary,
            "evidence_by_kind": evidence_by_kind(connection),
            "diagnostic_count": connection
                .query_row(
                    "SELECT count(*) FROM evidence WHERE kind = 'Diagnostic'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count diagnostics"),
        },
        "dependency_edges": {
            "resolved": resolved,
            "external_count": edge_count(connection, "External"),
            "unresolved_count": edge_count(connection, "Unresolved"),
            "unsupported_language_unresolved_count": unsupported_language_unresolved_count(connection),
        },
        "searchability": {
            "indexed_files_with_content": indexed_with_content(connection),
        },
    })
}

/// Asserts the published database matches `manifest` exactly.
fn assert_manifest(connection: &Connection, manifest: &Value) {
    assert_eq!(
        manifest["contract_version"].as_u64(),
        Some(CONTRACT_VERSION),
        "manifest contract_version must match the code's CONTRACT_VERSION"
    );
    assert_eq!(
        manifest["parser_pack_version"].as_str(),
        Some(PACK_VERSION),
        "manifest parser_pack_version must match parse::PACK_VERSION"
    );

    // Exact file set.
    let expected_paths: Vec<String> = manifest["file_set"]
        .as_array()
        .expect("file_set array")
        .iter()
        .map(|value| value.as_str().expect("path string").to_owned())
        .collect();
    assert_eq!(
        stored_paths(connection),
        expected_paths,
        "file set mismatch"
    );

    // Per-file records (including content hash, so fixture source edits
    // with identical evidence counts still fail the contract).
    let files = manifest["files"].as_object().expect("files object");
    assert_eq!(files.len(), expected_paths.len());
    for path in &expected_paths {
        let expected = &files[path];
        let actual = file_record(connection, path);
        assert_eq!(&actual, expected, "record mismatch for {path}");
    }

    // Totals.
    let totals = &manifest["totals"];
    let file_count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .expect("file count");
    assert_eq!(
        file_count,
        totals["file_count"].as_i64().expect("file_count")
    );
    assert_eq!(
        evidence_by_kind(connection),
        serde_json::from_value::<BTreeMap<String, i64>>(totals["evidence_by_kind"].clone())
            .expect("evidence_by_kind"),
        "evidence totals mismatch"
    );
    assert_eq!(
        totals["diagnostic_count"]
            .as_i64()
            .expect("diagnostic_count"),
        connection
            .query_row(
                "SELECT count(*) FROM evidence WHERE kind = 'Diagnostic'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count diagnostics")
    );

    // Dependency edges.
    let edges = &manifest["dependency_edges"];
    let expected_resolved: Vec<(String, String)> = edges["resolved"]
        .as_array()
        .expect("resolved array")
        .iter()
        .map(|pair| {
            let pair = pair.as_array().expect("edge pair");
            (
                pair[0].as_str().expect("from").to_owned(),
                pair[1].as_str().expect("to").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        resolved_edges(connection),
        expected_resolved,
        "resolved edges mismatch"
    );
    assert_eq!(
        edge_count(connection, "External"),
        edges["external_count"].as_i64().expect("external_count")
    );
    assert_eq!(
        edge_count(connection, "Unresolved"),
        edges["unresolved_count"]
            .as_i64()
            .expect("unresolved_count")
    );
    assert_eq!(
        unsupported_language_unresolved_count(connection),
        edges["unsupported_language_unresolved_count"]
            .as_i64()
            .expect("unsupported_language_unresolved_count")
    );

    // Searchability: every indexed file's content is stored and retrievable.
    let expected_indexed = totals["indexed"].as_i64().expect("indexed");
    assert_eq!(
        indexed_with_content(connection),
        expected_indexed,
        "every indexed file must have retrievable content"
    );
    assert_eq!(
        manifest["searchability"]["indexed_files_with_content"]
            .as_i64()
            .expect("searchability"),
        expected_indexed
    );
}

#[test]
fn the_checked_in_fixture_matches_its_golden_manifest() {
    let (_work, connection) = publish_fixture_copy();

    if std::env::var_os("SLUGAUDIT_REGEN_MANIFEST").is_some() {
        // Dump the raw dependency edges so the human reviewing a regenerated
        // manifest sees exactly how every import was classified.
        let mut statement = connection
            .prepare(
                "SELECT f.path, e.resolution_kind, e.confidence, COALESCE(t.path, '<none>'), e.raw_import_text \
                 FROM dependency_edges e \
                 JOIN files f ON f.id = e.from_file_id \
                 LEFT JOIN files t ON t.id = e.to_file_id \
                 ORDER BY f.path, e.resolution_kind",
            )
            .expect("prepare edge dump");
        let edges = statement
            .query_map([], |row| {
                Ok(format!(
                    "{} -> {} [{}] (conf {:?}) raw={:?}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .expect("query edge dump")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect edge dump");
        eprintln!("--- dependency edges ---");
        for edge in edges {
            eprintln!("{edge}");
        }

        let manifest = build_manifest(&connection);
        let formatted = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
        fs::write(manifest_path(), format!("{formatted}\n")).expect("write manifest");
        eprintln!(
            "!! SLUGAUDIT_REGEN_MANIFEST: {MANIFEST_FILE} was OVERWRITTEN with current output. \
             Review it by hand before committing — never commit a regenerated manifest unreviewed."
        );
        return;
    }

    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(manifest_path()).expect("read manifest"))
            .expect("parse manifest");
    assert_manifest(&connection, &manifest);
}
