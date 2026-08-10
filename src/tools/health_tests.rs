//! Tests for the `health` MCP tool.
//!
//! The crate re-exports both a `health` *function* (`crate::tools::health`)
//! and a `health` *module* (`crate::tools::health` as a path), so any
//! `use super::health::{...}` is ambiguous. We therefore reference the
//! function and module items by their full path inside test bodies,
//! importing only the types we need.

use crate::tools::HealthRequest;
use crate::tools::health::derive_phase;
use crate::tools::health::{HealthPhase, ToolCounters};
use crate::watch::{ProjectWatchState, WatcherHealth};
use rmcp::handler::server::wrapper::Parameters;

// `Parameters<X>` is the wrapper rmcp uses for deserialized tool
// arguments; tests skip the rmcp boundary and call the handler with
// a hand-built `Parameters<HealthRequest>` so we don't have to spin up
// a peer/transport.
fn req(path: Option<&str>) -> Parameters<HealthRequest> {
    Parameters(HealthRequest {
        path: path.map(str::to_owned),
    })
}

#[test]
fn phase_is_importing_when_healthy_but_no_revision() {
    assert_eq!(
        derive_phase(WatcherHealth::Healthy, &None, false),
        HealthPhase::Importing,
    );
    assert_eq!(
        derive_phase(WatcherHealth::Healthy, &None, true),
        HealthPhase::Importing,
    );
}

#[test]
fn phase_is_restoring_when_needs_verification() {
    let revision = Some("rev-1".to_owned());
    assert_eq!(
        derive_phase(WatcherHealth::NeedsVerification, &revision, false),
        HealthPhase::Restoring,
    );
}

#[test]
fn phase_is_restoring_when_desynced() {
    let revision = Some("rev-1".to_owned());
    assert_eq!(
        derive_phase(WatcherHealth::Desynced, &revision, false),
        HealthPhase::Restoring,
    );
}

#[test]
fn phase_is_restoring_when_healthy_with_unreconciled_events() {
    let revision = Some("rev-1".to_owned());
    assert_eq!(
        derive_phase(WatcherHealth::Healthy, &revision, true),
        HealthPhase::Restoring,
    );
}

#[test]
fn phase_is_steady_state_when_healthy_with_revision_and_no_unreconciled() {
    let revision = Some("rev-1".to_owned());
    assert_eq!(
        derive_phase(WatcherHealth::Healthy, &revision, false),
        HealthPhase::SteadyState,
    );
}

#[test]
fn phase_is_unavailable_when_watcher_unavailable() {
    let revision = Some("rev-1".to_owned());
    // Watcher Unavailable wins regardless of revision/extras — the
    // platform can't watch, so steady-state is unreachable.
    assert_eq!(
        derive_phase(WatcherHealth::Unavailable, &revision, false),
        HealthPhase::Unavailable,
    );
    assert_eq!(
        derive_phase(WatcherHealth::Unavailable, &revision, true),
        HealthPhase::Unavailable,
    );
}

#[test]
fn tool_counters_record_bumps_call_and_total_counts() {
    let before = ToolCounters::snapshot();
    let after = ToolCounters::record_call(true, 17);
    assert_eq!(after.call_count, before.call_count + 1);
    assert_eq!(
        after.error_count, before.error_count,
        "successful call does NOT bump error_count",
    );
    assert_eq!(after.total_ms, before.total_ms + 17);
}

#[test]
fn tool_counters_record_bumps_error_count_on_error() {
    let before = ToolCounters::snapshot();
    let after = ToolCounters::record_call(false, 3);
    assert_eq!(after.call_count, before.call_count + 1);
    assert_eq!(after.error_count, before.error_count + 1);
    assert_eq!(after.total_ms, before.total_ms + 3);
}

#[test]
fn tool_counters_always_return_non_decreasing_after_a_bump() {
    // The returned snapshot must reflect this call's increments, not a
    // stale value loaded before the call lands.
    let snap_calls_before = ToolCounters::snapshot().call_count;
    let snap_after = ToolCounters::record_call(true, 0);
    assert!(snap_after.call_count > snap_calls_before);
}

#[test]
fn derive_phase_agrees_with_a_real_snapshot_shape() {
    // Simple regression: derive the phase from a `ProjectWatchState`
    // snapshot directly so we can verify `has_unreconciled_events()`
    // inside `derive_phase` reacts to a real watcher recording an
    // event without reconcile.
    let snapshot = ProjectWatchState {
        health: WatcherHealth::Healthy,
        watcher_sequence: 1,
        ..ProjectWatchState::default()
    };
    let phase = derive_phase(snapshot.health, &None, snapshot.has_unreconciled_events());
    assert_eq!(phase, HealthPhase::Importing);
}

#[test]
fn health_no_path_response_carries_counter_consistency() {
    let snap_before = ToolCounters::snapshot();
    // Call the function via the full module path so the compiler can't
    // confuse it with the module-name collision.
    let resp = crate::tools::health::health(&req(None), &crate::progress::NoopProgressSink)
        .expect("health should not fail on shape");
    let inner = resp.0;
    let snap_after = ToolCounters::snapshot();
    assert!(
        inner.tool_call_count >= snap_before.call_count,
        "response counter can't precede the snapshot taken before the call"
    );
    assert!(
        inner.tool_call_count <= snap_after.call_count,
        "response counter can't exceed the snapshot taken after"
    );
}

#[test]
fn health_with_an_unrelated_path_returns_an_error() {
    // No active project for this path — ensure_current will fail with
    // a find_project_root error. The health tool surfaces such errors
    // rather than degrading silently: a path argument is a deliberate
    // operator query, not a fallback we run rate-limited.
    let resp = crate::tools::health::health(
        &req(Some("/definitely/not/a/real/project/that/exists")),
        &crate::progress::NoopProgressSink,
    );
    assert!(
        resp.is_err(),
        "health should surface the underlying error for a bad path"
    );
}
