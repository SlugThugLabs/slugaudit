//! Test-only fault injection for the writer: forces the failures that are
//! otherwise hard to cause (a full disk, a failure between batches, a result
//! that was never marked complete). In production builds the seam compiles to
//! the plain behaviour.

use super::error::ResultStoreError;
use super::writer::ResultWriter;

/// A fault the writer consults.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Fault {
    /// No fault.
    #[default]
    None,
    /// Lower `PRAGMA max_page_count` so SQLite itself raises SQLITE_FULL.
    MaxPages(u32),
    /// Fail at the n-th batch checkpoint (1-based), after that batch's rows.
    FailBeforeBatch(usize),
    /// Write `state='incomplete'` instead of `'complete'`, then register.
    IncompleteMeta,
}

#[cfg(test)]
impl ResultWriter<'_> {
    /// Arms `fault` on this writer.
    pub(crate) fn inject(&mut self, fault: Fault) -> Result<(), ResultStoreError> {
        if let Fault::MaxPages(pages) = fault {
            self.conn
                .query_row(&format!("PRAGMA max_page_count={pages}"), [], |_| Ok(()))
                .map_err(|error| ResultStoreError::from_sqlite("lower max_page_count", error))?;
        }
        self.fault = fault;
        Ok(())
    }

    pub(super) fn before_batch(&self) -> Result<(), ResultStoreError> {
        match self.fault {
            Fault::FailBeforeBatch(n) if n == self.batches => Err(ResultStoreError::unavailable(
                "injected batch failure",
                std::io::Error::other("injected"),
            )),
            _ => Ok(()),
        }
    }

    pub(super) fn final_state(&self) -> &'static str {
        match self.fault {
            Fault::IncompleteMeta => "incomplete",
            _ => "complete",
        }
    }
}

#[cfg(not(test))]
impl ResultWriter<'_> {
    pub(super) fn before_batch(&self) -> Result<(), ResultStoreError> {
        Ok(())
    }

    pub(super) fn final_state(&self) -> &'static str {
        "complete"
    }
}
