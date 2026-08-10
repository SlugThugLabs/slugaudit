use crate::tools;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Meta, ProgressNotificationParam, ProgressToken, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::Instrument;

use crate::tools::SyncRecencyCache;

const INSTRUCTIONS: &str = "SlugAudit supplies searchable, trustworthy evidence about a codebase — \
    parsed structure, symbols, imports, diagnostics, and prior AI-reviewed findings. Use `report` \
    for an automatic snapshot of what evidence exists, `query` for arbitrary read-only SQL against \
    the project's own database (search, symbol/import/diagnostic lookup, dependency traversal via \
    recursive CTEs over dependency_edges, and source retrieval all reach through it), `structure` \
    for Tree-sitter structural pattern matching, and `finding` to persist a conclusion you have \
    actually reviewed. Use `project_control` with `action` = `\"on\"` to enable a project (creates \
    the activation directory and runs the first import) or `\"off\"` to disable it. \
    SlugAudit performs no automated risk detection and reaches no conclusions itself: it supplies \
    evidence, the calling AI performs all judgment.";

/// Every tool handler does filesystem discovery, SQLite I/O, and (on first
/// sync) Tree-sitter parsing — all synchronous, potentially slow work that
/// would otherwise run directly on a Tokio worker thread and starve other
/// tasks scheduled on it. Bounds how many such blocking operations run at
/// once, independent of how many concurrent tool calls arrive.
const MAX_CONCURRENT_BLOCKING_OPS: usize = 8;

#[derive(Clone)]
pub struct SlugAuditServer {
    tool_router: ToolRouter<Self>,
    blocking_ops: Arc<Semaphore>,
    sync_recency: SyncRecencyCache,
}

impl SlugAuditServer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            blocking_ops: Arc::new(Semaphore::new(MAX_CONCURRENT_BLOCKING_OPS)),
            sync_recency: SyncRecencyCache::new(),
        }
    }
}

impl Default for SlugAuditServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `work` on Tokio's blocking thread pool rather than the calling
/// async task's worker thread, after acquiring a permit that caps how many
/// such operations run concurrently. `Semaphore::acquire` only errors if
/// the semaphore has been explicitly closed, which this server never does.
///
/// Wraps the whole call — permit wait, blocking work, and outcome — in one
/// span per tool call, logged to stderr only (see `main.rs`'s subscriber
/// setup): a start event, then completion/failure with elapsed time. Never
/// logs `work`'s actual arguments or return value — only the tool name and
/// timing/outcome, so SQL text, file content, and finding text never reach
/// the log. `work` itself may add its own fields to the current span (e.g.
/// revision id, row/edge counts) by calling `tracing::Span::current()`
/// from inside `tools::*` — the span is entered for the full duration of
/// the blocking closure via `blocking_span.enter()`, a sync guard, since
/// `spawn_blocking` runs on a thread the async instrumentation above does
/// not automatically follow.
///
/// When `progress` is `Some`, sends MCP progress notifications at three
/// points: "queued" before waiting on the semaphore, "working" once the
/// permit is acquired, and "completed" when the blocking work finishes
/// (whether it succeeded or failed). This lets the MCP consumer distinguish
/// a call that is queued behind the semaphore bound from one that is
/// actively running, rather than appearing frozen. The notifications are
/// best-effort: send failures are silently ignored so a broken progress
/// channel can never turn a successful tool call into an error.
async fn run_blocking<T: Send + 'static>(
    semaphore: Arc<Semaphore>,
    tool_name: &'static str,
    work: impl FnOnce() -> Result<T, ErrorData> + Send + 'static,
    progress: Option<(Peer<RoleServer>, ProgressToken)>,
) -> Result<T, ErrorData> {
    let span = tracing::info_span!("tool_call", tool = tool_name);
    let started = std::time::Instant::now();
    let blocking_span = span.clone();

    // Notify the consumer that the call has been received and is either
    // queued (waiting for a blocking permit) or about to start. Sending
    // this before the semaphore acquire is what makes a queued call
    // visible instead of silent.
    if let Some((ref peer, ref token)) = progress {
        let _ = peer
            .notify_progress(
                ProgressNotificationParam::new(token.clone(), 0.0)
                    .with_total(1.0)
                    .with_message(format!("{tool_name} queued")),
            )
            .await;
    }

    let result = async {
        tracing::info!("tool call started");
        // `acquire_owned` returns an `OwnedSemaphorePermit` that holds an
        // `Arc` clone of the semaphore, so the permit is `'static` and can
        // be moved into `spawn_blocking`. This matters for cancellation:
        // if the outer tool-call future is cancelled, the permit stays
        // held inside the (still-running) blocking task instead of being
        // returned to the pool prematurely — keeping the semaphore's bound
        // honest for the full duration of the work.
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

        // Now that we hold a permit, the call is actively running on a
        // blocking thread. Tell the consumer so it can update any visible
        // queue/progress state.
        if let Some((ref peer, ref token)) = progress {
            let _ = peer
                .notify_progress(
                    ProgressNotificationParam::new(token.clone(), 0.5)
                        .with_total(1.0)
                        .with_message(format!("{tool_name} working")),
                )
                .await;
        }

        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _guard = blocking_span.enter();
            work()
        })
        .await
        .map_err(|error| ErrorData::internal_error(format!("tool task failed: {error}"), None))?
    }
    .instrument(span.clone())
    .await;

    // Final progress notification: the work is done and the response is
    // about to be returned. Progress reaches 1.0 whether the work succeeded
    // or failed — "completed" describes the operation's lifecycle, not its
    // outcome.
    if let Some((ref peer, ref token)) = progress {
        let _ = peer
            .notify_progress(
                ProgressNotificationParam::new(token.clone(), 1.0)
                    .with_total(1.0)
                    .with_message(format!("{tool_name} completed")),
            )
            .await;
    }

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let _guard = span.enter();
    match &result {
        Ok(_) => tracing::info!(duration_ms = elapsed_ms, "tool call completed"),
        Err(error) => {
            tracing::warn!(duration_ms = elapsed_ms, error = %error.message, "tool call failed");
        }
    }
    result
}

#[tool_router]
impl SlugAuditServer {
    #[tool(
        description = "Automatic project snapshot: file/language counts, parser failures, evidence-kind counts, open findings. No score, no risk leads."
    )]
    async fn report(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::ReportRequest>,
    ) -> Result<Json<tools::ReportResponse>, ErrorData> {
        let cache = self.sync_recency.clone();
        let progress = progress_target(meta, &peer);
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "report",
            move || tools::report(&request, &cache),
            progress,
        )
        .await
    }

    #[tool(
        description = "Arbitrary read-only SQL against the project's own database. Search, symbol/import/diagnostic lookup, and source retrieval all reach through this one tool. Only writes are rejected, by the connection itself."
    )]
    async fn query(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::QueryRequest>,
    ) -> Result<Json<tools::QueryResponse>, ErrorData> {
        let cache = self.sync_recency.clone();
        let progress = progress_target(meta, &peer);
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "query",
            move || tools::query(&request, &cache),
            progress,
        )
        .await
    }

    #[tool(
        description = "Tree-sitter structural pattern matching against one file, for patterns normalized evidence and query can't easily express (e.g. an S-expression query for a specific AST shape)."
    )]
    async fn structure(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::StructureRequest>,
    ) -> Result<Json<tools::StructureResponse>, ErrorData> {
        let cache = self.sync_recency.clone();
        let progress = progress_target(meta, &peer);
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "structure",
            move || tools::structure(&request, &cache),
            progress,
        )
        .await
    }

    #[tool(
        description = "Persist a conclusion you have personally reviewed about a specific file \
         and line range. Supply `path` (any path in the active project, to select the database), \
         `file` (project-relative path of the file), `line_start`/`line_end` (one-based inclusive), \
         plus `severity`, `category`, `title`, and `description` (all your own judgment — SlugAudit \
         never generates these). The finding is bound to the file's current content hash and \
         auto-invalidates (status becomes stale) the moment that file changes, so findings never \
         outlive the source they were about. Use this to record reviewed issues, not raw diagnostics."
    )]
    async fn finding(
        &self,
        meta: Meta,
        peer: Peer<RoleServer>,
        request: Parameters<tools::FindingRequest>,
    ) -> Result<Json<tools::FindingResponse>, ErrorData> {
        let cache = self.sync_recency.clone();
        let progress = progress_target(meta, &peer);
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "finding",
            move || tools::finding(&request, &cache),
            progress,
        )
        .await
    }

    #[tool(
        description = "Enable or disable SlugAudit for a project. Pass `action` = `\"on\"` to enable \
         a project (creates the activation directory and runs the initial import), or `\"off\"` to \
         disable it (removes the activation directory and purges its database). Supply `path` to \
         target a specific project root; defaults to the current directory."
    )]
    async fn project_control(
        &self,
        _meta: Meta,
        _peer: Peer<RoleServer>,
        request: Parameters<tools::ProjectControlRequest>,
    ) -> Result<Json<tools::ProjectControlResponse>, ErrorData> {
        run_blocking(
            Arc::clone(&self.blocking_ops),
            "project_control",
            move || tools::project_control(&request),
            None,
        )
        .await
    }
}
fn progress_target(
    meta: Meta,
    peer: &Peer<RoleServer>,
) -> Option<(Peer<RoleServer>, ProgressToken)> {
    meta.get_progress_token().map(|token| (peer.clone(), token))
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SlugAuditServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(INSTRUCTIONS)
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
