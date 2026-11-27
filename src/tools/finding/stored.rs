//! Result-store row layout (`StoredFinding`), byte-budget row cutting, and
//! row-0 description clipping for `finding_read`.

use super::read::FindingReadEntry;
use crate::error_codes::store_error;
use crate::store::{Cell, Materialize, ResultStoreError, StoredRow};
use crate::tools::context::internal_error;
use crate::tools::paginate::{CutStep, byte_cut_step};
use rmcp::ErrorData;

pub(super) struct StoredFinding {
    pub(super) entry: FindingReadEntry,
}

impl Materialize for StoredFinding {
    fn columns() -> &'static [&'static str] {
        &[
            "id",
            "path",
            "line_start",
            "line_end",
            "severity",
            "category",
            "title",
            "description",
            "status",
            "source_hash",
        ]
    }

    fn to_values(&self) -> Vec<Cell> {
        let entry = &self.entry;
        vec![
            Cell::Int(entry.id),
            Cell::Text(entry.path.clone()),
            Cell::Int(i64::from(entry.line_start)),
            Cell::Int(i64::from(entry.line_end)),
            Cell::Text(entry.severity.clone()),
            Cell::Text(entry.category.clone()),
            Cell::Text(entry.title.clone()),
            Cell::Text(entry.description.clone()),
            Cell::Text(entry.status.clone()),
            Cell::Text(entry.source_hash.clone()),
        ]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        const LAYOUT: ResultStoreError = ResultStoreError::Invalid {
            reason: "finding_read row layout",
        };
        let line = |cell: &Cell| match cell {
            Cell::Int(value) => u32::try_from(*value).map_err(|_| LAYOUT),
            _ => Err(LAYOUT),
        };
        let text = |cell: &Cell| match cell {
            Cell::Text(value) => Ok(value.clone()),
            _ => Err(LAYOUT),
        };
        let [
            Cell::Int(id),
            path,
            line_start,
            line_end,
            severity,
            category,
            title,
            description,
            status,
            source_hash,
        ] = values
        else {
            return Err(LAYOUT);
        };
        Ok(Self {
            entry: FindingReadEntry {
                id: *id,
                path: text(path)?,
                line_start: line(line_start)?,
                line_end: line(line_end)?,
                severity: text(severity)?,
                category: text(category)?,
                title: text(title)?,
                description: text(description)?,
                detail_clipped: false,
                status: text(status)?,
                source_hash: text(source_hash)?,
            },
        })
    }
}

pub(super) fn cut_finding_row(
    row: &StoredRow,
    taken: usize,
    bytes_so_far: usize,
    budget: usize,
) -> Result<CutStep<FindingReadEntry>, ErrorData> {
    let mut entry = StoredFinding::from_values(&row.cells)
        .map_err(|e| store_error(&e))?
        .entry;
    let mut size = serialized_len(&entry)?;
    if taken == 0 && size > budget {
        clip_description(&mut entry, budget)?;
        size = serialized_len(&entry)?;
        return Ok(CutStep::Keep {
            row: entry,
            bytes: size,
        });
    }
    Ok(byte_cut_step(entry, size, taken, bytes_so_far, budget))
}

fn clip_description(entry: &mut FindingReadEntry, budget: usize) -> Result<(), ErrorData> {
    let chars: Vec<char> = entry.description.chars().collect();
    let original = entry.description.clone();
    entry.detail_clipped = true;
    let mut low = 0usize;
    let mut over = chars.len();
    if serialized_len(entry)? <= budget {
        low = over;
    }
    let mut fits_prefix = |len: usize| -> Result<bool, ErrorData> {
        entry.description = chars.iter().take(len).collect();
        Ok(serialized_len(entry)? <= budget)
    };
    while low.midpoint(over) > low {
        let mid = low.midpoint(over);
        if fits_prefix(mid)? {
            low = mid;
        } else {
            over = mid;
        }
    }
    let fits_low = fits_prefix(low)?;
    if low == chars.len() || (low == 0 && !fits_low) {
        entry.description = original;
        entry.detail_clipped = false;
    }
    Ok(())
}

fn serialized_len(entry: &FindingReadEntry) -> Result<usize, ErrorData> {
    crate::util::serialized_len(entry)
        .map_err(|error| internal_error("measuring a session finding's response size", error))
}
