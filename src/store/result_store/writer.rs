// slugaudit-line-exception: approved-by=slugthug; reason=one materialisation: spec and handle types, begin, push with non-finite marking, batch checkpoint and finish (meta last, commit, admit) share one writer state and one abort rule
//! Materialising one result: rows go into one private file inside ONE
//! transaction; `meta` is written last; only after the commit is the result
//! admitted and made resolvable. Any error drops the writer, closing the
//! connection of a file that is already unlinked, so nothing is left behind.
//!
//! `ROLLBACK` is deliberately not used: it is undefined under
//! `journal_mode=OFF`, and the file is discarded wholesale instead.

use super::cell::{Cell, Materialize, NonFinite};
use super::cursor::ResultId;
use super::error::ResultStoreError;
use super::registry::{Entry, ResultMeta};
use super::result_file::{self, MAX_COLUMNS};
use super::store::ResultStore;
use rusqlite::{Connection, ToSql};
use uuid::Uuid;

/// Rows between size checks (and fault-injection points).
const BATCH_ROWS: usize = 1_000;
/// Cell payload between size checks, so huge rows are measured sooner.
const BATCH_BYTES: usize = 16 * 1024 * 1024;

static NULL_CELL: Cell = Cell::Null;

/// What the tool tells the store about a result before its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultSpec {
    /// The producing tool.
    pub tool: String,
    /// The project label.
    pub project: String,
    /// The revision the rows were read at.
    pub revision_id: String,
    /// Column names (kept in `meta`; never put in SQL).
    pub columns: Vec<String>,
}

/// A finished, admitted result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultHandle {
    /// Its identity.
    pub id: ResultId,
    /// Rows stored.
    pub row_count: u64,
    /// Measured bytes on disk.
    pub bytes: u64,
}

/// An open materialisation. Dropping it without [`ResultWriter::finish`] is an
/// abort.
pub struct ResultWriter<'s> {
    store: &'s ResultStore,
    id: ResultId,
    pub(super) conn: Connection,
    spec: ResultSpec,
    insert_sql: String,
    next_ord: u64,
    rows_since_check: usize,
    bytes_since_check: usize,
    pub(super) batches: usize,
    failed: bool,
    #[cfg(test)]
    pub(super) fault: super::fault::Fault,
}

impl ResultStore {
    /// Starts a materialisation.
    ///
    /// # Errors
    ///
    /// `Invalid` for no columns or more than 1000; `Unavailable` when the
    /// file cannot be created.
    pub fn begin(&self, spec: ResultSpec) -> Result<ResultWriter<'_>, ResultStoreError> {
        if spec.columns.is_empty() || spec.columns.len() > MAX_COLUMNS {
            return Err(ResultStoreError::Invalid {
                reason: "a result needs between 1 and 1000 columns",
            });
        }
        let id = ResultId(Uuid::new_v4());
        let conn = result_file::create(self.dir(), self.session(), id.0)?;
        result_file::create_schema(&conn, spec.columns.len())?;
        conn.execute_batch("BEGIN").map_err(|error| {
            ResultStoreError::from_sqlite("begin the result transaction", error)
        })?;
        let insert_sql = format!(
            "INSERT INTO rows (ord, nonfinite, {}) VALUES (?{})",
            result_file::column_list(spec.columns.len()),
            (1..=spec.columns.len() + 2)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(", ?")
        );
        Ok(ResultWriter {
            store: self,
            id,
            conn,
            spec,
            insert_sql,
            next_ord: 0,
            rows_since_check: 0,
            bytes_since_check: 0,
            batches: 0,
            failed: false,
            #[cfg(test)]
            fault: super::fault::Fault::None,
        })
    }

    /// Materialises a typed sequence in one call.
    ///
    /// # Errors
    ///
    /// Anything [`ResultStore::begin`], [`ResultWriter::push`] and
    /// [`ResultWriter::finish`] report.
    pub fn write_all<T: Materialize>(
        &self,
        tool: &str,
        project: &str,
        revision_id: &str,
        rows: impl IntoIterator<Item = T>,
        completeness: &serde_json::Value,
    ) -> Result<ResultHandle, ResultStoreError> {
        let mut writer = self.begin(ResultSpec {
            tool: tool.to_owned(),
            project: project.to_owned(),
            revision_id: revision_id.to_owned(),
            columns: T::columns().iter().map(|name| (*name).to_owned()).collect(),
        })?;
        for row in rows {
            writer.push(&row.to_values())?;
        }
        writer.finish(completeness)
    }
}

impl ResultWriter<'_> {
    /// Adds one row. A NaN is stored as NULL and marked; infinities are stored
    /// natively and marked.
    ///
    /// # Errors
    ///
    /// `Invalid` for a wrong width; `TooLarge` once the file exceeds the whole
    /// budget; `Unavailable` for disk or I/O failures. After an error the
    /// writer refuses further use.
    pub fn push(&mut self, cells: &[Cell]) -> Result<(), ResultStoreError> {
        self.guard()?;
        let outcome = self.insert(cells);
        self.failed = outcome.is_err();
        outcome
    }

    fn guard(&self) -> Result<(), ResultStoreError> {
        if self.failed {
            return Err(ResultStoreError::Invalid {
                reason: "the writer already failed",
            });
        }
        Ok(())
    }

    fn insert(&mut self, cells: &[Cell]) -> Result<(), ResultStoreError> {
        if cells.len() != self.spec.columns.len() {
            return Err(ResultStoreError::Invalid {
                reason: "row width differs from the column count",
            });
        }
        let marks: Vec<(usize, NonFinite)> = cells
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| match cell {
                Cell::Real(value) => NonFinite::of(*value).map(|mark| (index, mark)),
                _ => None,
            })
            .collect();
        let nonfinite = encode_marks(&marks)?;
        let ord = i64::try_from(self.next_ord).map_err(|_| ResultStoreError::Invalid {
            reason: "too many rows",
        })?;
        let mut params: Vec<&dyn ToSql> = vec![&ord, &nonfinite];
        params.extend(cells.iter().map(|cell| -> &dyn ToSql {
            match cell {
                Cell::Real(value) if value.is_nan() => &NULL_CELL,
                other => other,
            }
        }));
        self.conn
            .prepare_cached(&self.insert_sql)
            .and_then(|mut statement| statement.execute(params.as_slice()))
            .map_err(|error| ResultStoreError::from_sqlite("insert a result row", error))?;
        self.next_ord += 1;
        self.rows_since_check += 1;
        self.bytes_since_check += cells.iter().map(Cell::payload_len).sum::<usize>();
        if self.rows_since_check >= BATCH_ROWS || self.bytes_since_check >= BATCH_BYTES {
            self.checkpoint()?;
        }
        Ok(())
    }

    /// After each batch: consult the fault seam, then abort once this file
    /// alone exceeds the whole budget.
    fn checkpoint(&mut self) -> Result<(), ResultStoreError> {
        self.batches += 1;
        self.rows_since_check = 0;
        self.bytes_since_check = 0;
        self.before_batch()?;
        let bytes = result_file::measure_bytes(&self.conn)?;
        if bytes > self.store.limits().max_bytes {
            self.store.refuse_too_large(self.id, bytes);
            return Err(ResultStoreError::TooLarge);
        }
        Ok(())
    }

    /// Writes `meta` last, commits, measures, then admits and registers.
    ///
    /// # Errors
    ///
    /// `TooLarge`, `Full`, or `Unavailable`; on any of them nothing is
    /// registered and no cursor exists.
    pub fn finish(
        mut self,
        completeness: &serde_json::Value,
    ) -> Result<ResultHandle, ResultStoreError> {
        self.guard()?;
        let created_unix = self.store.now();
        let meta = ResultMeta {
            tool: self.spec.tool.clone(),
            project: self.spec.project.clone(),
            revision_id: self.spec.revision_id.clone(),
            columns: self.spec.columns.clone(),
            completeness: completeness.clone(),
            row_count: self.next_ord,
            created_unix,
        };
        self.write_meta(&meta)?;
        self.conn
            .execute_batch("COMMIT")
            .map_err(|error| ResultStoreError::from_sqlite("commit the result", error))?;
        let bytes = result_file::measure_bytes(&self.conn)?;
        let handle = ResultHandle {
            id: self.id,
            row_count: meta.row_count,
            bytes,
        };
        let entry = Entry::new(self.conn, meta, bytes, created_unix);
        self.store.admit(self.id, entry)?;
        Ok(handle)
    }

    fn write_meta(&mut self, meta: &ResultMeta) -> Result<(), ResultStoreError> {
        let columns = serde_json::to_string(&meta.columns)
            .map_err(|error| ResultStoreError::json("encode the column names", error))?;
        let completeness = serde_json::to_string(&meta.completeness)
            .map_err(|error| ResultStoreError::json("encode the completeness", error))?;
        let row_count = i64::try_from(meta.row_count).map_err(|_| ResultStoreError::Invalid {
            reason: "too many rows",
        })?;
        let state = self.final_state();
        let put = |key: &str, value: &dyn ToSql| {
            self.conn
                .execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                    rusqlite::params![key, value],
                )
                .map(drop)
                .map_err(|error| ResultStoreError::from_sqlite("write result metadata", error))
        };
        put("tool", &meta.tool)?;
        put("project", &meta.project)?;
        put("revision_id", &meta.revision_id)?;
        put("columns", &columns)?;
        put("completeness", &completeness)?;
        put("created_unix", &meta.created_unix)?;
        put("row_count", &row_count)?;
        put("bytes", &0_i64)?;
        put("state", &state)?;
        // The size is only known once everything else is in place.
        let bytes = i64::try_from(result_file::measure_bytes(&self.conn)?).map_err(|_| {
            ResultStoreError::Invalid {
                reason: "result too large to measure",
            }
        })?;
        self.conn
            .execute("UPDATE meta SET value = ?1 WHERE key = 'bytes'", [bytes])
            .map_err(|error| ResultStoreError::from_sqlite("write result metadata", error))?;
        Ok(())
    }
}

/// `[[col, "NaN"], ...]` or NULL when every cell is finite.
fn encode_marks(marks: &[(usize, NonFinite)]) -> Result<Option<String>, ResultStoreError> {
    if marks.is_empty() {
        return Ok(None);
    }
    let named: Vec<(usize, &str)> = marks.iter().map(|(i, m)| (*i, m.as_str())).collect();
    serde_json::to_string(&named)
        .map(Some)
        .map_err(|error| ResultStoreError::json("encode non-finite marks", error))
}
