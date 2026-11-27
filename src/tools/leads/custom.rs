//! Single-pass registered-mechanic scan inside `leads`: tallies counts,
//! optionally collects rows in catalogue order, and feeds them into `Feed`.

use super::coverage::LanguagesRead;
use super::stored::Feed;
use super::{CompactLeadRow, LeadMode, LeadRow, MAX_DETAIL_CHARS, MechanicCount, clip_detail};
use crate::tools::context::internal_error;
use crate::tools::mechanic::catalog::Catalog;
use crate::tools::mechanic::scan::{self as mechanic_scan, ScanEnd};
use crate::tools::page::Completeness;
use rmcp::ErrorData;
use rusqlite::Transaction;
use std::time::Instant;

/// One entry's complete count under a pattern.
#[derive(Debug, Clone, Default)]
struct Tally {
    count: i64,
    files: i64,
    last_file: Option<String>,
}

impl Tally {
    fn record(&mut self, file: &str) -> Result<(), ErrorData> {
        let overflow =
            |what: &str| internal_error("counting mechanic hits", format!("{what} overflowed"));
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| overflow("the hit count"))?;
        if self.last_file.as_deref() != Some(file) {
            self.files = self
                .files
                .checked_add(1)
                .ok_or_else(|| overflow("the file count"))?;
            self.last_file = Some(file.to_owned());
        }
        Ok(())
    }
}

struct RetainedHit {
    file: String,
    line: i64,
    text: String,
}

/// Output of one custom-mechanic scan pass over the current transaction.
pub(super) struct CustomScan {
    pub(super) counts: Vec<MechanicCount>,
    rows_by_entry: Vec<Vec<RetainedHit>>,
    pub(super) completeness: Completeness,
}

impl CustomScan {
    pub(super) fn feed_entry(
        &self,
        catalog: &Catalog,
        index: usize,
        mode: LeadMode,
        feed: &mut Feed<'_, '_, '_>,
    ) -> Result<(), ErrorData> {
        let registered = catalog
            .custom
            .get(index)
            .ok_or_else(|| index_out_of_range("custom", index, catalog.custom.len()))?;
        let held = self
            .rows_by_entry
            .get(index)
            .ok_or_else(|| index_out_of_range("rows_by_entry", index, self.rows_by_entry.len()))?;
        let id = registered.entry.id.as_str();
        for hit in held {
            match mode {
                LeadMode::Full => feed.full(full_row(&hit.file, hit.line, &hit.text))?,
                LeadMode::Compact => feed.compact(CompactLeadRow {
                    mechanic: id.to_owned(),
                    file: hit.file.clone(),
                    line: hit.line,
                })?,
            }
        }
        Ok(())
    }
}

pub(super) fn scan_entries(
    tx: &Transaction<'_>,
    catalog: &Catalog,
    scanned: &[usize],
    retain_rows: bool,
    glob: Option<&str>,
    deadline: Option<Instant>,
    read: &LanguagesRead,
) -> Result<CustomScan, ErrorData> {
    let entry_len = catalog.custom.len();
    let mut tallies = vec![Tally::default(); entry_len];
    let mut rows_by_entry: Vec<Vec<RetainedHit>> = (0..entry_len).map(|_| Vec::new()).collect();
    let end = if scanned.is_empty() {
        ScanEnd::Finished
    } else {
        mechanic_scan::scan(tx, &catalog.custom, scanned, glob, deadline, &mut |hit| {
            tallies
                .get_mut(hit.entry)
                .ok_or_else(|| index_out_of_range("tallies", hit.entry, entry_len))?
                .record(hit.file)?;
            if retain_rows {
                rows_by_entry
                    .get_mut(hit.entry)
                    .ok_or_else(|| index_out_of_range("rows_by_entry", hit.entry, entry_len))?
                    .push(RetainedHit {
                        file: hit.file.to_owned(),
                        line: hit.line,
                        text: hit.text.to_owned(),
                    });
            }
            Ok(())
        })?
    };
    let counts = build_counts(catalog, scanned, &tallies, read)?;
    let completeness = match end {
        ScanEnd::Finished => Completeness::Complete,
        ScanEnd::Stopped(stop) => Completeness::Partial {
            limited_by: stop.limited_by,
            resume_after_file: stop.resume_after_file,
            files_scanned: stop.files_scanned,
            files_total: stop.files_total,
        },
    };
    Ok(CustomScan {
        counts,
        rows_by_entry,
        completeness,
    })
}

fn build_counts(
    catalog: &Catalog,
    scanned: &[usize],
    tallies: &[Tally],
    read: &LanguagesRead,
) -> Result<Vec<MechanicCount>, ErrorData> {
    scanned
        .iter()
        .map(|&index| {
            let registered = catalog
                .custom
                .get(index)
                .ok_or_else(|| index_out_of_range("custom", index, catalog.custom.len()))?;
            let tally = tallies
                .get(index)
                .ok_or_else(|| index_out_of_range("tallies", index, tallies.len()))?;
            Ok(MechanicCount {
                mechanic: registered.entry.id.clone(),
                source: registered.source,
                note: Some(registered.entry.note.clone()),
                count: tally.count,
                files: tally.files,
                covers: read.covers_language(&registered.entry.language),
            })
        })
        .collect()
}

fn full_row(file: &str, line: i64, text: &str) -> LeadRow {
    let mut row = LeadRow {
        file: file.to_owned(),
        line,
        detail: text.chars().take(MAX_DETAIL_CHARS + 1).collect(),
        detail_clipped: false,
    };
    clip_detail(&mut row);
    row
}

pub(super) fn index_out_of_range(what: &str, index: usize, len: usize) -> ErrorData {
    internal_error(
        "indexing custom mechanics",
        format!("{what} index {index} is out of range for length {len}"),
    )
}
