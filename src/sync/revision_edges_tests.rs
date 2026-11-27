// slugaudit-line-exception: approved-by=split-judge; reason=transactional integration tests and EdgeTestHarness covering incremental dependency_edges resolution and blast-radius invalidation
use super::*;
use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, ExtractionCompleteness, ImportPayload,
    ParseOutcome, ParserAvailability, ParserRun, RustModPathPayload, SourceIdentity,
    SpanAvailability, StructurePayload,
};
use crate::store::open_read_write;
use crate::sync::revision::{RevisionInputs, publish_revision};
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

fn file_record(
    path: &str,
    hash: &str,
    content: &str,
    language: Option<&str>,
    run: ParserRun,
    evidence: Vec<EvidenceItem>,
) -> FileRecord {
    FileRecord {
        relative_path: path.to_owned(),
        is_binary: false,
        content: Some(content.to_owned()),
        identity: SourceIdentity {
            relative_path: path.to_owned(),
            content_hash: hash.to_owned(),
            hash_algorithm: "blake3".to_owned(),
        },
        byte_len: content.len() as u64,
        modified_unix_seconds: None,
        language: language.map(str::to_owned),
        language_detected: language.is_some(),
        run,
        evidence,
    }
}

fn rust_record(path: &str, hash: &str, evidence: Vec<EvidenceItem>) -> FileRecord {
    file_record(
        path,
        hash,
        "",
        Some("rust"),
        ParserRun {
            availability: ParserAvailability::Available,
            outcome: ParseOutcome::Succeeded,
            completeness: ExtractionCompleteness::Full,
        },
        evidence,
    )
}

fn lib_record() -> FileRecord {
    rust_record("src/lib.rs", "hl1", vec![])
}

fn import_evidence(index: usize, source: &str) -> EvidenceItem {
    EvidenceItem {
        key: format!("import:{index}"),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: EvidencePayload::Import(ImportPayload {
            source: source.to_owned(),
            items: Vec::new(),
            alias: None,
            is_wildcard: source.contains('*'),
        }),
    }
}

fn mod_evidence(index: usize, name: &str, path_attr: Option<&str>) -> EvidenceItem {
    EvidenceItem {
        key: format!("structure:{index}"),
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: EvidencePayload::Structure(StructurePayload {
            kind: "Module".to_owned(),
            name: Some(name.to_owned()),
            visibility: None,
            decorators: Vec::new(),
            signature: None,
            doc_comment: None,
            rust_mod_path: Some(RustModPathPayload {
                path_attr: path_attr.map(str::to_owned),
                inline_dir: String::new(),
                inline_depth: 0,
                inline_segments: Vec::new(),
            }),
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

impl StoredEdge {
    fn resolved(from: &str, to: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: Some(to.to_owned()),
            kind: "Resolved".to_owned(),
            syntax_unmodeled: false,
        }
    }

    fn unresolved(from: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: None,
            kind: "Unresolved".to_owned(),
            syntax_unmodeled: false,
        }
    }

    fn external(from: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: None,
            kind: "External".to_owned(),
            syntax_unmodeled: false,
        }
    }

    fn unmodeled(from: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: None,
            kind: "Unresolved".to_owned(),
            syntax_unmodeled: true,
        }
    }
}

struct EdgeTestHarness {
    _db_dir: tempfile::TempDir,
    conn: Connection,
    rev: Option<String>,
    step: usize,
    live_paths: HashSet<String>,
}

impl EdgeTestHarness {
    fn new() -> TestResult<Self> {
        let db_dir = tempfile::tempdir().check("create temporary db directory")?;
        let conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;
        Ok(Self {
            _db_dir: db_dir,
            conn,
            rev: None,
            step: 0,
            live_paths: HashSet::new(),
        })
    }

    fn publish(&mut self, upserts: &[FileRecord], re_resolve: bool) -> TestResult {
        self.publish_with_deletions(upserts, &[], re_resolve)
    }

    fn publish_with_deletions(
        &mut self,
        upserts: &[FileRecord],
        deletions: &[String],
        re_resolve: bool,
    ) -> TestResult {
        self.step = self.step.checked_add(1).check("step counter fits")?;
        for path in deletions {
            self.live_paths.remove(path);
        }
        for file in upserts {
            self.live_paths.insert(file.relative_path.clone());
        }
        let manifest = format!("m{}", self.step);
        let unrepresentable = HashSet::new();
        let next = publish_revision(
            &mut self.conn,
            &RevisionInputs {
                expected_current: self.rev.as_deref(),
                manifest_hash: &manifest,
                parser_pack_version: "1.0.0",
                upserts,
                deletions,
                skipped: &[],
                discovered_paths: &self.live_paths,
                re_resolve_unresolved: re_resolve,
                unrepresentable_paths: &unrepresentable,
                deadline: None,
            },
        )
        .check("publish revision")?;
        self.rev = Some(next);
        Ok(())
    }

    fn edge_row_ids_for(&self, from_path: &str) -> TestResult<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT e.id FROM dependency_edges e \
                 JOIN files f ON f.id = e.from_file_id \
                 WHERE f.path = ?1 ORDER BY e.id",
            )
            .check("prepare edge row id query")?;
        stmt.query_map([from_path], |row| row.get(0))
            .check("query edge row ids")?
            .collect::<Result<_, _>>()
            .check("collect edge row ids")
    }

    fn assert_edges(&self, expected: &[StoredEdge], context: &str) -> TestResult {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.path, t.path, e.resolution_kind, e.syntax_unmodeled \
                 FROM dependency_edges e \
                 JOIN files f ON f.id = e.from_file_id \
                 LEFT JOIN files t ON t.id = e.to_file_id \
                 ORDER BY f.path, COALESCE(t.path, ''), e.resolution_kind",
            )
            .check("prepare edge query")?;
        let actual: Vec<StoredEdge> = stmt
            .query_map([], |row| {
                Ok(StoredEdge {
                    from: row.get(0)?,
                    to: row.get(1)?,
                    kind: row.get(2)?,
                    syntax_unmodeled: row.get(3)?,
                })
            })
            .check("run edge query")?
            .collect::<Result<_, _>>()
            .check("collect stored edges")?;
        ensure_eq(&actual.as_slice(), &expected, context)
    }
}

fn foo_tests_super() -> FileRecord {
    rust_record(
        "src/foo_tests.rs",
        "ht1",
        vec![import_evidence(0, "use super::*;")],
    )
}

fn foo_with_path(hash: &str) -> FileRecord {
    rust_record(
        "src/foo.rs",
        hash,
        vec![mod_evidence(0, "tests", Some("foo_tests.rs"))],
    )
}

fn foo_path_harness() -> TestResult<EdgeTestHarness> {
    let mut h = EdgeTestHarness::new()?;
    h.publish(
        &[foo_tests_super(), lib_record(), foo_with_path("hf1")],
        false,
    )?;
    Ok(h)
}

#[test]
fn grouped_use_resolves_incrementally_across_revisions_without_pop_loss() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let caller = rust_record(
        "src/caller.rs",
        "h1",
        vec![
            import_evidence(0, "use crate::{a, b};"),
            import_evidence(1, "use crate::foo::{};"),
            import_evidence(2, "pubfn use broken;"),
        ],
    );
    h.publish(&[caller, rust_record("src/a.rs", "h2", vec![])], false)?;
    h.assert_edges(
        &[
            StoredEdge::unresolved("src/caller.rs"),
            StoredEdge::unmodeled("src/caller.rs"),
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
        ],
        "initial publish emits resolved a.rs, unresolved b, and unmodeled broken edge",
    )?;

    h.publish(&[rust_record("src/b.rs", "h3", vec![])], true)?;
    h.assert_edges(
        &[
            StoredEdge::unmodeled("src/caller.rs"),
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
            StoredEdge::resolved("src/caller.rs", "src/b.rs"),
        ],
        "incremental publish re-resolves unchanged caller.rs to both a.rs and b.rs",
    )
}

#[test]
fn adding_rust_module_upgrades_low_confidence_crate_root_edge() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let caller = rust_record(
        "src/caller.rs",
        "hc1",
        vec![import_evidence(0, "use crate::{a, b};")],
    );
    h.publish(
        &[lib_record(), caller, rust_record("src/a.rs", "ha1", vec![])],
        false,
    )?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
            StoredEdge::resolved("src/caller.rs", "src/lib.rs"),
        ],
        "while b.rs is absent, crate::b falls back to the crate root with Low confidence",
    )?;

    h.publish(&[rust_record("src/b.rs", "hb1", vec![])], true)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/caller.rs", "src/a.rs"),
            StoredEdge::resolved("src/caller.rs", "src/b.rs"),
        ],
        "adding b.rs upgrades unchanged caller.rs edge from lib.rs to b.rs",
    )
}

#[test]
fn path_attr_super_edges_update_when_host_added_or_modified() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    h.publish(&[foo_tests_super(), lib_record()], false)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/foo_tests.rs", "src/lib.rs")],
        "without a declaring file, super::* falls back to the crate root",
    )?;

    h.publish(&[rust_record("src/foo.rs", "hf1", vec![])], true)?;
    h.publish(&[foo_with_path("hf2")], false)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/foo.rs", "src/foo_tests.rs"),
            StoredEdge::resolved("src/foo_tests.rs", "src/foo.rs"),
        ],
        "modifying foo.rs to declare #[path] emits mod edge and re-resolves foo_tests.rs to foo.rs",
    )
}

#[test]
fn editing_rust_file_with_unchanged_path_attr_does_not_reresolve_dependents() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let other_super = rust_record(
        "src/other.rs",
        "ho1",
        vec![import_evidence(0, "use super::*;")],
    );
    h.publish(
        &[
            lib_record(),
            foo_with_path("hf1"),
            foo_tests_super(),
            other_super,
        ],
        false,
    )?;
    let foo_tests_ids_before = h.edge_row_ids_for("src/foo_tests.rs")?;
    let other_ids_before = h.edge_row_ids_for("src/other.rs")?;

    // Edit foo.rs body (new hash) while keeping the exact same #[path = "foo_tests.rs"] mod tests;
    h.publish(&[foo_with_path("hf2")], false)?;

    ensure_eq(
        &h.edge_row_ids_for("src/foo_tests.rs")?,
        &foo_tests_ids_before,
        "unchanged #[path] attribute on foo.rs does not rewrite child foo_tests.rs edges",
    )?;
    ensure_eq(
        &h.edge_row_ids_for("src/other.rs")?,
        &other_ids_before,
        "unchanged #[path] attribute on foo.rs does not trigger full-repo super::* re-resolution",
    )
}

#[test]
fn adding_or_deleting_non_rust_file_does_not_reresolve_rust_dependents() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let caller = rust_record(
        "src/caller.rs",
        "hc1",
        vec![
            import_evidence(0, "use crate::missing_mod::Item;"),
            import_evidence(1, "use super::*;"),
        ],
    );
    h.publish(&[lib_record(), caller], false)?;
    let caller_ids_before = h.edge_row_ids_for("src/caller.rs")?;

    let notes = file_record(
        "NOTES.md",
        "hn1",
        "# notes\n",
        Some("prose"),
        ParserRun {
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::NotAttempted,
            completeness: ExtractionCompleteness::ContentOnly,
        },
        Vec::new(),
    );
    h.publish(&[notes], true)?;
    ensure_eq(
        &h.edge_row_ids_for("src/caller.rs")?,
        &caller_ids_before,
        "adding NOTES.md does not re-resolve Rust crates or low-confidence edges",
    )?;

    h.publish_with_deletions(&[], &["NOTES.md".to_owned()], true)?;
    ensure_eq(
        &h.edge_row_ids_for("src/caller.rs")?,
        &caller_ids_before,
        "deleting NOTES.md does not re-resolve Rust crates or low-confidence edges",
    )
}

#[test]
fn path_attr_super_edges_revert_when_host_drops_attr() -> TestResult {
    let mut h = foo_path_harness()?;
    h.publish(&[rust_record("src/foo.rs", "hf2", vec![])], false)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/foo_tests.rs", "src/lib.rs")],
        "removing #[path] from foo.rs clears mod edge and reverts foo_tests.rs to lib.rs",
    )
}

#[test]
fn path_attr_super_edges_become_unresolved_when_host_and_root_deleted() -> TestResult {
    let mut h = foo_path_harness()?;
    h.publish_with_deletions(
        &[],
        &["src/foo.rs".to_owned(), "src/lib.rs".to_owned()],
        true,
    )?;
    h.assert_edges(
        &[StoredEdge::unresolved("src/foo_tests.rs")],
        "deleting the declaring file and crate root reverts child edge to Unresolved",
    )
}

#[test]
fn removing_path_attr_updates_child_super_super_edge_pointing_elsewhere() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let parent_with_path = rust_record(
        "src/a/parent.rs",
        "hp1",
        vec![mod_evidence(0, "tests", Some("../child.rs"))],
    );
    let child = rust_record(
        "src/child.rs",
        "hc1",
        vec![import_evidence(0, "use super::super::target::Item;")],
    );
    let target_under_a = rust_record("src/a/target.rs", "ht1", vec![]);
    h.publish(&[parent_with_path, child, target_under_a], false)?;
    h.assert_edges(
        &[
            StoredEdge::resolved("src/a/parent.rs", "src/child.rs"),
            StoredEdge::resolved("src/child.rs", "src/a/target.rs"),
        ],
        "while parent.rs declares child.rs via #[path], parent->child and child->target both resolve",
    )?;

    h.publish(&[rust_record("src/a/parent.rs", "hp2", vec![])], false)?;
    h.assert_edges(
        &[StoredEdge::unresolved("src/child.rs")],
        "when parent.rs drops #[path] while child.rs is unchanged, stale super::super edge is cleared",
    )
}

#[test]
fn external_mod_declarations_emit_edges_and_reresolve_when_target_added() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    h.publish(
        &[rust_record(
            "src/lib.rs",
            "hl1",
            vec![mod_evidence(0, "sub", None)],
        )],
        false,
    )?;
    h.assert_edges(
        &[StoredEdge::unresolved("src/lib.rs")],
        "external mod sub; is Unresolved before src/sub.rs exists",
    )?;

    h.publish(&[rust_record("src/sub.rs", "hs1", vec![])], true)?;
    h.assert_edges(
        &[StoredEdge::resolved("src/lib.rs", "src/sub.rs")],
        "adding src/sub.rs upgrades unchanged lib.rs mod sub; edge to Resolved",
    )
}

fn toml_record(path: &str, hash: &str, content: &str) -> FileRecord {
    file_record(
        path,
        hash,
        content,
        None,
        ParserRun {
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::NotAttempted,
            completeness: ExtractionCompleteness::Unavailable,
        },
        Vec::new(),
    )
}

#[test]
fn workspace_cargo_toml_resolves_cross_crate_imports_and_updates_on_edit() -> TestResult {
    let mut h = EdgeTestHarness::new()?;
    let alpha_toml = toml_record(
        "crates/alpha/Cargo.toml",
        "ht1",
        "[package]\nname = \"my-alpha\"\nversion = \"0.1.0\"\n",
    );
    let alpha_lib = rust_record("crates/alpha/src/lib.rs", "ha1", vec![]);
    let beta_lib = rust_record(
        "crates/beta/src/lib.rs",
        "hb1",
        vec![import_evidence(0, "use my_alpha::Widget;")],
    );
    h.publish(&[alpha_toml, alpha_lib, beta_lib], false)?;
    h.assert_edges(
        &[StoredEdge::resolved(
            "crates/beta/src/lib.rs",
            "crates/alpha/src/lib.rs",
        )],
        "hyphenated workspace package my-alpha resolves use my_alpha::Widget",
    )?;

    let renamed_toml = toml_record(
        "crates/alpha/Cargo.toml",
        "ht2",
        "[package]\nname = \"renamed-alpha\"\nversion = \"0.1.0\"\n",
    );
    h.publish(&[renamed_toml], false)?;
    h.assert_edges(
        &[StoredEdge::external("crates/beta/src/lib.rs")],
        "editing Cargo.toml in place downgrades missing workspace crate to External",
    )?;

    let restored_toml = toml_record(
        "crates/alpha/Cargo.toml",
        "ht3",
        "[package]\nname = \"my-alpha\"\nversion = \"0.1.0\"\n",
    );
    h.publish(&[restored_toml], false)?;
    h.assert_edges(
        &[StoredEdge::resolved(
            "crates/beta/src/lib.rs",
            "crates/alpha/src/lib.rs",
        )],
        "editing Cargo.toml back upgrades External cross-crate edge to Resolved",
    )
}
