//! Proofs that the result store is independent of `project.db`: each way the
//! database is replaced at runtime (stale root, incompatible version,
//! corruption recovery, `project_control off`) runs while a result is open,
//! and the continuation still returns the remaining pages exactly as a read
//! taken before the replacement. A full publish and a WAL truncate also
//! succeed with results open. Real manager, real paginate, no mocks.

use crate::progress::NoopProgressSink;
use crate::store::{Cell, Materialize, ResultStoreError, ResultStoreLimits, StoredRow, Take};
use crate::sync::SourceSyncManager;
use crate::tools::{
    Completeness, PageRequest, PageSpec, Produced, ProjectControlAction, ProjectControlRequest,
    Served, decode_rows, paginate, project_control,
};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const TOOL: &str = "proof";
const ROWS: i64 = 30;
const PAGE_ROWS: usize = 10;

#[derive(Debug, Clone, PartialEq)]
struct Item(i64);

impl Materialize for Item {
    fn columns() -> &'static [&'static str] {
        &["n"]
    }
    fn to_values(&self) -> Vec<Cell> {
        vec![Cell::Int(self.0)]
    }
    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        match values {
            [Cell::Int(n)] => Ok(Self(*n)),
            _ => Err(ResultStoreError::Invalid {
                reason: "proof row layout",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Totals {
    produced: u64,
}

/// What a page shows a caller, comparable across reads.
#[derive(Debug, PartialEq)]
struct Seen {
    rows: Vec<i64>,
    returned: usize,
    total: Option<u64>,
    complete: bool,
    has_cursor: bool,
}

fn seen(served: &Served<Totals>) -> Seen {
    Seen {
        rows: decode_rows::<Item>(&served.rows)
            .expect("proof rows decode")
            .into_iter()
            .map(|Item(n)| n)
            .collect(),
        returned: served.page.returned,
        total: served.page.total,
        complete: served.page.complete,
        has_cursor: served.page.cursor.is_some(),
    }
}

fn per_page() -> impl FnMut(&StoredRow) -> Take {
    let mut taken = 0;
    move |_| {
        taken += 1;
        if taken <= PAGE_ROWS {
            Take::Yes
        } else {
            Take::No
        }
    }
}

fn manager() -> SourceSyncManager {
    SourceSyncManager::with_result_store_limits(ResultStoreLimits {
        max_bytes: 64 * 1024 * 1024,
        active_grace_secs: 900,
    })
}

fn open_first_page(manager: &SourceSyncManager, path: &str) -> Served<Totals> {
    paginate(
        manager,
        &PageSpec::typed::<Item>(TOOL, 1000),
        PageRequest::Fresh {
            path,
            sink: &NoopProgressSink,
        },
        |_, sink| {
            for n in 0..ROWS {
                sink.push(&Item(n))?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: Totals {
                    produced: ROWS.unsigned_abs(),
                },
            })
        },
        |_| per_page(),
    )
    .expect("materialise the result and serve page one")
}

fn continue_with(manager: &SourceSyncManager, token: &str) -> Result<Served<Totals>, ErrorData> {
    paginate(
        manager,
        &PageSpec::typed::<Item>(TOOL, 1000),
        PageRequest::Cursor(token),
        |_, _| panic!("a continuation never produces"),
        |_| per_page(),
    )
}

/// Walks the continuation from `token` to the last page.
fn remaining_pages(manager: &SourceSyncManager, token: &str) -> Vec<Seen> {
    let mut pages = Vec::new();
    let mut token = token.to_owned();
    loop {
        let served = continue_with(manager, &token).expect("continuation page");
        let next = served.page.cursor.clone();
        pages.push(seen(&served));
        match next {
            Some(next) => token = next,
            None => return pages,
        }
    }
}

struct Open {
    project: tempfile::TempDir,
    path: String,
    manager: SourceSyncManager,
    token: String,
    before: Vec<Seen>,
}

impl Open {
    fn database(&self) -> PathBuf {
        self.project.path().join(".planning/slugaudit/project.db")
    }

    fn resync(&self) {
        self.manager
            .ensure_current(&self.path, &NoopProgressSink)
            .expect("the sync recovers by replacing the database");
    }

    /// Reads the remaining pages again and compares them with the read taken
    /// before the replacement.
    fn assert_continuation_unchanged(&self) {
        assert_eq!(self.before.len(), 2, "pages two and three exist");
        assert_eq!(remaining_pages(&self.manager, &self.token), self.before);
    }
}

/// A synced project with one result whose first page was served.
fn open_result() -> Open {
    let project = crate::tools::test_support::activated_project("lib.rs", b"pub fn a() {}\n");
    let path = project.path().to_string_lossy().into_owned();
    let manager = manager();
    let first = open_first_page(&manager, &path);
    assert_eq!(seen(&first).rows, (0..10).collect::<Vec<_>>());
    let token = first.page.cursor.clone().expect("more pages remain");
    let before = remaining_pages(&manager, &token);
    Open {
        project,
        path,
        manager,
        token,
        before,
    }
}

fn database_rows(open: &Open, sql: &str) -> String {
    crate::store::open_read_only(&open.database())
        .expect("open the index")
        .query_row(sql, [], |row| row.get(0))
        .expect("read the index")
}

#[test]
fn result_store_proof_stale_root_replacement_keeps_the_open_result_readable() {
    let open = open_result();
    {
        let connection = crate::store::open_read_write(&open.database()).expect("open the index");
        connection
            .execute(
                "UPDATE project SET root_path = '/elsewhere/moved' WHERE id = 1",
                [],
            )
            .expect("stamp a stale root");
    }

    open.resync();

    assert_ne!(
        database_rows(&open, "SELECT root_path FROM project WHERE id = 1"),
        "/elsewhere/moved",
        "the stale database was replaced"
    );
    open.assert_continuation_unchanged();
}

#[test]
fn result_store_proof_incompatible_version_replacement_keeps_the_open_result_readable() {
    let open = open_result();
    {
        let connection = crate::store::open_read_write(&open.database()).expect("open the index");
        connection
            .execute(
                "UPDATE project SET contract_version = 99, schema_version = 99 WHERE id = 1",
                [],
            )
            .expect("stamp an unsupported version");
    }

    open.resync();

    assert_ne!(
        database_rows(
            &open,
            "SELECT CAST(contract_version AS TEXT) FROM project WHERE id = 1"
        ),
        "99",
        "the incompatible database was replaced"
    );
    open.assert_continuation_unchanged();
}

#[test]
fn result_store_proof_corruption_recovery_keeps_the_open_result_readable() {
    let open = open_result();
    std::fs::write(open.database(), b"this is not a sqlite database").expect("corrupt the index");

    open.resync();

    assert_eq!(
        database_rows(&open, "SELECT path FROM files WHERE path = 'lib.rs'"),
        "lib.rs",
        "the corrupt database was replaced by a rebuilt one"
    );
    open.assert_continuation_unchanged();
}

#[test]
fn result_store_proof_project_control_off_keeps_the_open_result_readable() {
    let open = open_result();

    let response = project_control(
        &Parameters(ProjectControlRequest {
            path: Some(open.path.clone()),
            action: ProjectControlAction::Off,
            profile: None,
        }),
        &NoopProgressSink,
        Some(&open.manager),
    )
    .expect("project_control off succeeds with a result open");

    assert_eq!(response.0.status, "disabled");
    assert!(
        !open.project.path().join(".planning/slugaudit").exists(),
        "the activation directory, results directory included, is gone"
    );
    open.assert_continuation_unchanged();
}

#[test]
fn result_store_proof_publish_and_wal_checkpoint_truncate_with_results_open() {
    let open = open_result();
    let second = open_first_page(&open.manager, &open.path);
    assert!(second.page.cursor.is_some(), "the second result is open");
    std::fs::write(
        open.project.path().join("lib.rs"),
        b"pub fn a() {}\npub fn b() {}\n",
    )
    .expect("change a source file");
    let mut connection = crate::store::open_read_write(&open.database()).expect("open the index");

    crate::sync::publish_cas::publish(
        &mut connection,
        open.project.path(),
        "1.0",
        &NoopProgressSink,
    )
    .expect("a full publish succeeds with results open");
    let (busy, log, checkpointed): (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("checkpoint the WAL");

    assert_eq!(
        (busy, log, checkpointed),
        (0, 0, 0),
        "the checkpoint ran to completion"
    );
    let wal = open.database().with_extension("db-wal");
    assert_eq!(
        std::fs::metadata(&wal).expect("the WAL file exists").len(),
        0,
        "the WAL truncated to zero bytes"
    );
    open.assert_continuation_unchanged();
}
