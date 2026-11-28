use crate::store::open_read_write;
use crate::sync::FileRecord;
use crate::sync::revision::{RevisionInputs, SkippedRecord, publish_revision};
use rusqlite::Connection;
use slugaudit_test_check::{Check, TestResult, ensure_eq};
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct StoredEdge {
    from: String,
    to: Option<String>,
    kind: String,
    syntax_unmodeled: bool,
}

impl StoredEdge {
    fn new(from: &str, to: Option<&str>, kind: &str, syntax_unmodeled: bool) -> Self {
        Self {
            from: from.to_owned(),
            to: to.map(str::to_owned),
            kind: kind.to_owned(),
            syntax_unmodeled,
        }
    }

    pub(super) fn resolved(from: &str, to: &str) -> Self {
        Self::new(from, Some(to), "Resolved", false)
    }

    pub(super) fn unresolved(from: &str) -> Self {
        Self::new(from, None, "Unresolved", false)
    }

    pub(super) fn external(from: &str) -> Self {
        Self::new(from, None, "External", false)
    }

    pub(super) fn unmodeled(from: &str) -> Self {
        Self::new(from, None, "Unresolved", true)
    }
}

pub(super) struct EdgeTestHarness {
    _db_dir: tempfile::TempDir,
    conn: Connection,
    rev: Option<String>,
    live_paths: HashSet<String>,
}

impl EdgeTestHarness {
    pub(super) fn new() -> TestResult<Self> {
        let db_dir = tempfile::tempdir().check("create temporary db directory")?;
        let conn = open_read_write(&db_dir.path().join("project.db")).check("open database")?;
        Ok(Self {
            _db_dir: db_dir,
            conn,
            rev: None,
            live_paths: HashSet::new(),
        })
    }

    pub(super) fn connection(&self) -> &Connection {
        &self.conn
    }

    pub(super) fn publish(&mut self, upserts: &[FileRecord], re_resolve: bool) -> TestResult {
        self.publish_with_deletions(upserts, &[], re_resolve)
    }

    pub(super) fn publish_with_deletions(
        &mut self,
        upserts: &[FileRecord],
        deletions: &[String],
        re_resolve: bool,
    ) -> TestResult {
        self.publish_with_skipped(upserts, deletions, &[], re_resolve)
    }

    pub(super) fn publish_with_skipped(
        &mut self,
        upserts: &[FileRecord],
        deletions: &[String],
        skipped: &[SkippedRecord],
        re_resolve: bool,
    ) -> TestResult {
        for path in deletions {
            self.live_paths.remove(path);
        }
        for file in upserts {
            self.live_paths.insert(file.relative_path.clone());
        }
        for skip in skipped {
            self.live_paths.insert(skip.relative_path.clone());
        }
        let unrepresentable = HashSet::new();
        let next = publish_revision(
            &mut self.conn,
            &RevisionInputs {
                expected_current: self.rev.as_deref(),
                parser_pack_version: "1.0.0",
                upserts,
                deletions,
                skipped,
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

    pub(super) fn edge_row_ids_for(&self, from_path: &str) -> TestResult<Vec<i64>> {
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

    #[track_caller]
    pub(super) fn assert_edge_row_ids_unchanged(
        &self,
        path: &str,
        before: &[i64],
        msg: &str,
    ) -> TestResult {
        ensure_eq(&self.edge_row_ids_for(path)?.as_slice(), &before, msg)
    }

    pub(super) fn assert_edges(&self, expected: &[StoredEdge], context: &str) -> TestResult {
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
