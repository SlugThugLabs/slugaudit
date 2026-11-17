//! How a finished structure scan is reported: its completeness and the extras
//! stored beside the matches.

use super::page::{Completeness, PageLimit};
use super::structure::stored::{CutFacts, StructureExtra};
use super::structure_page::RunOutput;
use super::structure_source::SourceWindow;
use rmcp::ErrorData;

/// How far the scan got, as stored with the result: `Partial` when the
/// execution budget cancelled a parse (resume after that file, which would
/// only time out again) or the memory window ended before the matching files
/// did (resume after the last file examined).
///
/// # Errors
///
/// An internal error when the file counts do not add up in `usize`.
pub(super) fn completeness_of(
    window: &SourceWindow,
    run: &RunOutput,
) -> Result<Completeness, ErrorData> {
    let files_total = window
        .files
        .len()
        .checked_add(window.files_over_parse_limit)
        .and_then(|seen| seen.checked_add(window.files_not_scanned))
        .ok_or_else(|| {
            ErrorData::internal_error("structure: the scanned-file count overflowed", None)
        })? as u64;
    let files_scanned = run.files_scanned as u64;
    if let Some(file) = &run.parse_timed_out_file {
        return Ok(Completeness::Partial {
            limited_by: PageLimit::TimeBudget,
            // KEEP clone: completeness borrows &RunOutput and returns an owned Completeness
            resume_after_file: Some(file.clone()),
            files_scanned,
            files_total,
        });
    }
    if window.files_not_scanned > 0 {
        return Ok(Completeness::Partial {
            limited_by: PageLimit::ScanWindow,
            // KEEP clone: completeness borrows &SourceWindow and returns an owned Completeness
            resume_after_file: window.next_start_after_file.clone(),
            files_scanned,
            files_total,
        });
    }
    Ok(Completeness::Complete)
}

/// The extras of a finished scan.
pub(super) fn extra_of(
    language: String,
    window: SourceWindow,
    run: RunOutput,
    cut: CutFacts,
) -> StructureExtra {
    StructureExtra {
        cut,
        language,
        files_not_scanned: window.files_not_scanned,
        bytes_not_scanned: window.bytes_not_scanned,
        next_start_after_file: run
            .parse_timed_out_file
            // KEEP clone: parse_timed_out_file is also moved into StructureExtra below
            .clone()
            .or(window.next_start_after_file),
        unparsed_files: run.unparsed_files,
        files_over_parse_limit: window.files_over_parse_limit,
        parse_timed_out_file: run.parse_timed_out_file,
    }
}
