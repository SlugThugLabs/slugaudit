//! Request execution pipeline, grammar/query compilation, and [`Completeness`]
//! / [`StructureExtra`] reporting for `structure`.

use super::StructureRequest;
use super::match_node::OutputMode;
use super::runner::{RunOutput, run_query};
use super::source::{SourceWindow, WindowBounds, load_targets};
use super::stored::StructureExtra;
use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::model::ResourceLimits;
use crate::tools::grammar::{load_grammar, map_grammar_load_error};
use crate::tools::page::{Completeness, PageLimit};
use crate::tools::paginate::{Produced, RowSink};
use rmcp::ErrorData;
use rusqlite::Transaction;
use tree_sitter::Query;

pub(super) fn produce(
    tx: &Transaction<'_>,
    rows: &mut RowSink<'_, '_>,
    request: &StructureRequest,
    limits: &ResourceLimits,
    window_bytes: usize,
) -> Result<Produced<StructureExtra>, ErrorData> {
    let bounds = WindowBounds {
        window_bytes,
        max_parse_bytes: limits.max_parse_bytes,
    };
    let (target_language, window) = load_targets(tx, request, bounds)?;
    let (ts_language, compiled_query) = compile_query(&target_language, &request.query)?;
    let single_file = request.file.is_some();
    let mode = OutputMode {
        full_text: match request.full_text {
            Some(full_text) => full_text,
            None => single_file,
        },
        single_file,
    };
    let run = run_query(
        &compiled_query,
        &ts_language,
        &window.files,
        limits,
        &mode,
        rows,
    )?;
    let (completeness, next_start_after_file) = completeness_and_resume(&window, &run)?;
    Ok(Produced {
        completeness,
        extra: StructureExtra {
            language: target_language,
            files_not_scanned: window.files_not_scanned,
            bytes_not_scanned: window.bytes_not_scanned,
            next_start_after_file,
            unparsed_files: run.unparsed_files,
            files_over_parse_limit: window.files_over_parse_limit,
            parse_timed_out_file: run.parse_timed_out_file,
        },
    })
}

fn completeness_and_resume(
    window: &SourceWindow,
    run: &RunOutput,
) -> Result<(Completeness, Option<String>), ErrorData> {
    let files_total = window
        .files
        .len()
        .checked_add(window.files_over_parse_limit)
        .and_then(|seen| seen.checked_add(window.files_not_scanned))
        .ok_or_else(|| {
            ErrorData::internal_error("structure: the scanned-file count overflowed", None)
        })? as u64;
    let files_scanned = run.files_scanned as u64;
    let partial = |limited_by: PageLimit, resume: Option<String>| {
        (
            Completeness::Partial {
                limited_by,
                resume_after_file: resume.clone(),
                files_scanned,
                files_total,
            },
            resume,
        )
    };
    if let Some(file) = run
        .parse_timed_out_file
        .as_deref()
        .or(run.budget_resume_after.as_deref())
    {
        return Ok(partial(PageLimit::TimeBudget, Some(file.to_owned())));
    }
    if run.query_match_limit_hit {
        return Ok(partial(
            PageLimit::QueryMatchLimit,
            run.match_limit_resume_after.clone(),
        ));
    }
    if window.files_not_scanned > 0 {
        return Ok(partial(
            PageLimit::ScanWindow,
            window.next_start_after_file.clone(),
        ));
    }
    Ok((Completeness::Complete, None))
}

fn compile_query(language: &str, text: &str) -> Result<(tree_sitter::Language, Query), ErrorData> {
    let ts_language =
        load_grammar(language).map_err(|error| map_grammar_load_error(language, None, error))?;
    let compiled_query = Query::new(&ts_language, text).map_err(|error| {
        invalid_params_with_code(
            format!("invalid tree-sitter query syntax: {error}"),
            ErrorCode::InvalidTreeSitterQuery,
        )
    })?;
    Ok((ts_language, compiled_query))
}
