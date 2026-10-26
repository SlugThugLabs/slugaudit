use super::context::{ensure_synced, internal_error, with_verified_read};
use super::mechanic::{InvalidMechanic, MechanicListing};
use super::mechanic_catalog;
#[cfg(test)]
use super::report_sections::MAX_PARSER_FAILURE_FILES;
use super::report_sections::build_report;
use crate::progress::ProgressSink;
use crate::sync;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rusqlite::Transaction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_kind` value.
pub struct FileKindCount {
    /// One of `indexed`, `binary`, `skipped` — `files.file_kind`.
    pub kind: String,
    /// Number of `files` rows carrying this `file_kind`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one `files.file_class` value.
pub struct FileClassCount {
    /// One of `authored`, `generated`, `vendored` — `files.file_class`.
    pub class: String,
    /// Number of `files` rows carrying this `file_class`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one recorded skip reason.
pub struct SkipReasonCount {
    /// A short stable code from `files.skip_reason`: `oversize` (over the
    /// per-file import ceiling), `unreadable` (the read failed) or
    /// `stat_failed` (the stat failed, so `files.byte_len` is NULL: size
    /// unknown, not 0). Empty when a skipped row carries no recorded reason.
    pub reason: String,
    /// Number of skipped `files` rows with this `reason`.
    pub count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// Row count for one extraction-completeness value.
pub struct CompletenessCount {
    /// One of `Full`, `Partial`, `ContentOnly`, `Unavailable`. `ContentOnly`
    /// means the file's text is in `files.content` but no parser ran over it
    /// — the file is readable even though it has no structural evidence.
    /// `Unavailable` means no content is retained at all (binary, or skipped).
    pub extraction_completeness: String,
    /// Number of `files` rows with this `extraction_completeness` value.
    pub count: i64,
}

/// Counts that say whether the rest of the response is the whole truth.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompletenessEnvelope {
    /// Every `files` row by `file_kind`. `indexed` is the set with content and
    /// metadata — its structural evidence may still be missing, which
    /// `indexed_files_without_evidence` counts; `binary` was sampled and
    /// deliberately not parsed; `skipped` was discovered but never read, so
    /// it has no content.
    pub file_kind_counts: Vec<FileKindCount>,
    /// Every `files` row by `file_class` — where the code came from, derived
    /// from the path alone (`sync::classify`): `authored` (the project's own
    /// source), `generated` (a machine-written file: a `generated` path token
    /// or a codegen suffix such as `.pb.go` / `.g.dart` / `.min.js`), or
    /// `vendored` (a third-party tree: `vendor/`, `node_modules/`, …). A
    /// mechanic whose rows are dominated by one class can be summarised in a
    /// sentence instead of paged row by row. `vendored` subsumes `generated`
    /// when a path matches both. A path heuristic, not a verdict: it says
    /// where a file came from, never whether it matters.
    pub file_class_counts: Vec<FileClassCount>,
    /// `files` rows with `file_kind = 'skipped'`, i.e. files that exist in the
    /// project but are not in the index.
    pub skipped_file_count: i64,
    /// Those skipped rows grouped by `skip_reason`. This is the "why" paired
    /// with `index_partial`. A file with no representable `files.path` (a
    /// non-UTF-8 name) cannot appear here; it is counted by
    /// `unrepresentable_path_count` instead.
    pub skipped_by_reason: Vec<SkipReasonCount>,
    /// Every `files` row by `extraction_completeness`, so "text present but
    /// not parsed" (`ContentOnly`) is visible as itself instead of vanishing
    /// into a parser-failure count.
    pub extraction_completeness_counts: Vec<CompletenessCount>,
    /// True when the index does not cover every file discovery found in the
    /// current tree: an existence row is `skipped`, or a discovered file's
    /// path has no UTF-8 representation and therefore no `files` row at all.
    /// False means every discovered file is represented by a row here.
    pub index_partial: bool,
    /// Discovered files whose project-relative path is not valid UTF-8, so
    /// no `files` row can represent them and they cannot be stored, skipped
    /// or named in this report. Sync persists this set during every full
    /// discovery and refreshes it from every incremental stat sweep, so the
    /// count is read from the index rather than from a walk run for this
    /// report. It is project-level — describing the tree as of the sync that
    /// produced this report, not a particular revision — and is included in
    /// `index_partial`.
    pub unrepresentable_path_count: i64,
    /// Files whose stored evidence was cut short by an evidence cap: each
    /// carries the `evidence:truncated` diagnostic appended by
    /// `sync::sample::apply_evidence_limits`. Their structural record is
    /// incomplete in a way this report cannot recover, so an agent counting
    /// `evidence_counts` for them is counting a floor, not a total.
    pub evidence_capped_file_count: i64,
    /// `indexed` rows with no evidence at all — content is present and
    /// readable, but the index holds nothing structural about them (a
    /// language with no loaded parser, or a file that parsed to nothing).
    /// Binary and skipped rows are excluded: no evidence is their expected
    /// state, not a gap.
    pub indexed_files_without_evidence: i64,
    /// `indexed` files stored as `language = 'prose'` (Markdown,
    /// reStructuredText, AsciiDoc, Org, plain text): content is stored and
    /// searchable, but no grammar is run on them by design, so they are
    /// `ContentOnly`, have no evidence, and are counted in
    /// `indexed_files_without_evidence`, yet they are not parser failures and
    /// never appear in `parser_failure_*`. Subtract this from those counts to
    /// see the files that lack evidence for another reason.
    pub prose_files_not_parsed: i64,
    /// `indexed` files larger than `max_parse_bytes` (`parse_error_reason =
    /// 'oversize_for_parse'`): content is stored and searchable via `query`,
    /// but no grammar was run, so they have no evidence and every `leads`
    /// mechanic that reads evidence is blind to them. They are also listed
    /// among the parser failures and counted in `leads.files_not_parsed`.
    pub oversize_for_parse_file_count: i64,
    /// `indexed` files a parse of which was cut at the sync deadline
    /// (`parse_error_reason = 'parse_timeout'`): content is stored; no
    /// evidence when the main parse was cut (`ContentOnly`), the main
    /// parse's evidence but no call or binding evidence when a later
    /// re-parse was (`Partial`). Also listed among the parser failures and
    /// counted in `leads.files_not_parsed`.
    pub parse_timeout_file_count: i64,
}

/// Edge totals in both directions, over the whole revision.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyEdgeTotals {
    /// `count(*) FROM dependency_edges`: every edge, since every edge
    /// originates at a file (`from_file_id` is `NOT NULL`).
    pub outbound_edge_count: i64,
    /// Edges whose `to_file_id` names a row in `files` — the subset that
    /// lands on a project file. `Unresolved` and `External` edges carry no
    /// target, and `import_resolution` splits those two apart; this total
    /// groups them only by their absence of a target.
    pub inbound_edge_count: i64,
    /// Distinct files that are the source of at least one edge.
    pub outbound_file_count: i64,
    /// Distinct files that are the target of at least one edge.
    pub inbound_file_count: i64,
}

/// Builds the edge totals for the current revision.
pub(super) fn build_dependency_edges(
    connection: &Transaction<'_>,
) -> Result<DependencyEdgeTotals, rusqlite::Error> {
    let outbound_edge_count: i64 =
        connection.query_row("SELECT count(*) FROM dependency_edges", [], |row| {
            row.get(0)
        })?;
    let inbound_edge_count: i64 = connection.query_row(
        "SELECT count(*) FROM dependency_edges WHERE to_file_id IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let outbound_file_count: i64 = connection.query_row(
        "SELECT count(DISTINCT from_file_id) FROM dependency_edges",
        [],
        |row| row.get(0),
    )?;
    let inbound_file_count: i64 = connection.query_row(
        "SELECT count(DISTINCT to_file_id) FROM dependency_edges",
        [],
        |row| row.get(0),
    )?;
    Ok(DependencyEdgeTotals {
        outbound_edge_count,
        inbound_edge_count,
        outbound_file_count,
        inbound_file_count,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
/// The project to snapshot — any path inside an active project.
pub struct ReportRequest {
    /// Any path inside the active project: its root, or a file/directory under it.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
/// How many discovered files the last revision attributed to one detected
/// language.
pub struct LanguageCount {
    pub language: String,
    pub file_count: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
/// How many evidence rows the current revision holds for one kind.
pub struct EvidenceKindCount {
    pub kind: String,
    pub count: i64,
}

/// How many import edges fell into each resolution bucket. A project with
/// mostly `External` and `Unresolved` edges and very few `Resolved` ones
/// has an import graph the evidence can't connect — useful context for an
/// AI deciding how much to trust dependency traversal for this project.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportResolutionCount {
    /// One of `Resolved`, `Unresolved`, or `External`.
    pub kind: String,
    pub count: i64,
}

/// A file whose parse failed or was not attempted, with enough detail for
/// an AI to decide whether the failure is worth investigating or just
/// noise (e.g. a `.claude_output.txt` scratch file that should have been
/// excluded, vs. a real source file with a genuine syntax error).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ParserFailureFile {
    pub path: String,
    pub parse_outcome: String,
    /// Why no tree was extracted: the grammar's load error, the parser's
    /// hard error (not just syntax error nodes), or a stable code —
    /// `oversize_for_parse` (content over `max_parse_bytes`, never parsed) or
    /// `parse_timeout` (the parse was cancelled at the deadline). `None` when
    /// no grammar exists for the file or for `SyntaxErrors`.
    pub parse_error_reason: Option<String>,
    /// One of `Full`, `Partial`, `ContentOnly`, `Unavailable`. `ContentOnly`
    /// means the file's text *is* indexed but no parser ran over it — the
    /// row is listed for availability reasons, not because extraction
    /// failed, and its content can still be read from `files.content`.
    pub extraction_completeness: String,
    /// One of `Available`, `Unavailable`, `LoadFailed`. Separates
    /// `ContentOnly` rows that have no parser for their language
    /// (`Unavailable`) from ones whose parser could not be loaded
    /// (`LoadFailed`), which a completeness value alone cannot.
    pub parser_availability: String,
}

const MAX_PATH_BYTES: usize = 32 * 1024;

/// Automatic project snapshot: what evidence exists, not what's suspicious
/// about it. No score, no risk leads, no safe/unsafe claim.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReportResponse {
    /// Revision id of the index this snapshot was read from.
    pub revision_id: String,
    /// Rows in `files`: every discovered file whose project-relative path is
    /// representable as UTF-8, whether or not it was indexed. A discovered
    /// path that is not valid UTF-8 cannot have a row and is counted by
    /// `completeness.unrepresentable_path_count` instead. Read
    /// `completeness.file_kind_counts` to see how many of these rows have
    /// content (`indexed`), are binary, or were skipped — this number alone
    /// does not mean "files with evidence".
    pub file_count: i64,
    /// Whether every count above is the whole truth: rows by kind, skipped
    /// rows by reason, rows by extraction completeness, and a partial-index
    /// flag. See [`CompletenessEnvelope`].
    pub completeness: CompletenessEnvelope,
    /// Detected language of each `files` row, with its file count; grouped by
    /// language and ordered by language name. Rows with no detected language
    /// are not listed.
    pub languages: Vec<LanguageCount>,
    /// Evidence rows by `kind` across the current revision, ordered by kind.
    pub evidence_counts: Vec<EvidenceKindCount>,
    /// Indexed files whose parser was unavailable or whose parse failed — the
    /// same predicate `parser_failure_files` lists, without the list cap.
    pub parser_failure_count: i64,
    /// Up to `MAX_PARSER_FAILURE_FILES` files with parse failures, so the
    /// AI can see whether failures are concentrated in scratch/temp files
    /// (noise) or in real source files (worth investigating).
    pub parser_failure_files: Vec<ParserFailureFile>,
    /// Uncapped total of files matching the `parser_failure_files` predicate.
    /// Larger than the returned list's length means the list was truncated at
    /// `MAX_PARSER_FAILURE_FILES`; equal means the list is complete.
    pub parser_failure_files_total: i64,
    /// Rows in `findings` with `status = 'current'`: findings whose file
    /// content hash still matches the hash they were written against.
    /// Findings invalidated by a later content change are `stale` and are not
    /// counted.
    pub open_finding_count: i64,
    /// How many import edges are Resolved vs Unresolved vs External — a
    /// key asymmetry signal. A project with mostly External/Unresolved
    /// imports has a disconnected import graph that dependency traversal
    /// can't help with.
    pub import_resolution: Vec<ImportResolutionCount>,
    /// The same graph counted in both directions: how many edges leave a
    /// file, how many land on one, and how many files are on each end.
    /// Pure arithmetic over `dependency_edges` — no ranking, no severity,
    /// and no claim about which file matters. Per-file traversal is the
    /// `slugaudit://queries/blast_radius.sql` recipe.
    pub dependency_edges: DependencyEdgeTotals,
    /// Total number of Diagnostic evidence items (e.g. linter or compiler
    /// diagnostics extracted from source files).
    pub diagnostic_count: i64,
    /// Of the `Unresolved` count in `import_resolution`, how many come from
    /// import syntax the resolver couldn't parse at all — `extract_reference`
    /// returned `None` on the raw text (e.g. Go's `import "fmt"`), as
    /// opposed to an import whose syntax was parsed but matched no project
    /// file (e.g. a stdlib module like `kotlin.math.max`). SlugAudit
    /// records both as `Unresolved` edges since it can't tell them apart
    /// at resolution time, but they mean very different things: a high
    /// count here means "this project's import syntax isn't modeled yet,"
    /// not "this project's imports are broken."
    pub unsupported_language_unresolved_count: i64,
    /// How many rows in `files` match common credential-file patterns
    /// (`.env`, `*.pem`, `*.key`, `id_rsa*`, etc. — the same list the
    /// SQL below matches, and the same list `leads`' `credential_files`
    /// rows use). SlugAudit records these paths like any other file —
    /// including when the file was skipped and no content was retained —
    /// so this count makes the AI aware it is handling credential-*looking*
    /// paths even when it cannot read them.
    pub credential_pattern_file_count: i64,
    /// Every mechanic `leads` counts — session entries, then project_config
    /// entries, then the built-ins — with its source and the author's note,
    /// plus the registered entries that failed to load. On the first
    /// `report` of this server only, `note` also explains how mechanics
    /// work.
    pub mechanics: ReportMechanics,
    /// `mechanics.invalid.len()`: registered entries that failed to load and
    /// are not counted by `leads` until fixed or removed.
    pub mechanics_invalid: usize,
}

/// The one-time note the first `report` of a server carries.
const MECHANICS_NOTE: &str = "The built-in mechanics are counting primitives, not a checklist. \
    What counts as a signal in this codebase is your decision. Register one with `mechanic` \
    action `add` (`id`, `language`, `query` with exactly one capture, `note`); `leads` then \
    counts and pages it beside the built-ins. Session entries last until this server exits; \
    `persist: true` writes the entry to .planning/slugaudit/config.json for later sessions.";

/// The mechanics section of a report.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ReportMechanics {
    /// Present on the first `report` of this server only; `null` after.
    pub note: Option<String>,
    /// Every loaded mechanic: session, then project_config, then builtin.
    pub entries: Vec<MechanicListing>,
    /// Registered entries that failed to load, each with its error. Never
    /// dropped: a persisted entry with a bad shape, an unknown section
    /// `version`, a compile error or an id collision stays listed here.
    pub invalid: Vec<InvalidMechanic>,
}

/// # Errors
///
/// Returns an error if `request.path` isn't inside an active project, or if
/// syncing or querying the project's database fails.
pub fn report(
    request: &Parameters<ReportRequest>,
    sink: &dyn ProgressSink,
    manager: &sync::SourceSyncManager,
) -> Result<Json<ReportResponse>, ErrorData> {
    if crate::util::exceeds_input_bytes(&request.0.path, MAX_PATH_BYTES) {
        return Err(ErrorData::invalid_params("path exceeds 32768 bytes", None));
    }
    let synced = ensure_synced(&request.0.path, sink, manager)?;
    let revision_id = synced.revision_id.clone();
    let mut response = with_verified_read(&synced, |tx| {
        build_report(tx, revision_id.clone())
            .map_err(|error| internal_error("building the report", error))
    })?;
    let root = super::project_root(&request.0.path)?;
    let session = manager.session_mechanics();
    let catalog = mechanic_catalog::load(&root, session, &super::limits_for_root(&root));
    response.mechanics_invalid = catalog.invalid.len();
    response.mechanics = ReportMechanics {
        note: None,
        entries: catalog.listings(),
        invalid: catalog.invalid,
    };
    // Claimed last, once nothing else can fail, so a failed first report
    // does not use up the note.
    if session.claim_report_note() {
        response.mechanics.note = Some(MECHANICS_NOTE.to_owned());
    }
    Ok(Json(response))
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "report_prose_tests.rs"]
mod prose_tests;
