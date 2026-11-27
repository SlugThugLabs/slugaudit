#![forbid(unsafe_code)]

//! End-to-end Rust workspace dogfood test exercising `project_control`,
//! `report`, `leads`, `structure`, every shipped SQL recipe under
//! `examples/queries/*.sql`, and Rust CST evidence + cross-crate graph accuracy.

use std::fs;
use std::path::Path;

use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::sync::SourceSyncManager;
use slugaudit_mcp_rust::tools::{
    LeadsRequest, ProjectControlRequest, ProjectControlStatus, QueryRequest, ReportRequest,
    StructureRequest, leads, project_control, query, report, structure,
};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

const ROOT_CARGO_TOML: &str = r#"[package]
name = "dogfood_cli"
version = "0.1.0"
edition = "2024"

[workspace]
members = ["crates/core", "crates/app"]

[dependencies]
dogfood_core = { path = "crates/core" }
"#;

const ROOT_MAIN_RS: &str = r#"mod runner;

use dogfood_core::{AuthService, open_session};

pub fn main() {
    let _svc = AuthService { active: true };
    let _len = runner::run_cli(open_session(("alice", "tok-1")));
}
"#;

const ROOT_RUNNER_RS: &str = r"pub fn run_cli(session_id: String) -> usize {
    session_id.len()
}
";

const CORE_CARGO_TOML: &str = r#"[package]
name = "dogfood-core"
version = "0.1.0"
edition = "2024"
"#;

const CORE_LIB_RS: &str = r#"//! Core authentication crate inner module docs — must never leak onto item docs.

/// Authenticates incoming requests against active sessions.
pub trait Authenticator {
    /// Validates an opaque session token without a default body.
    fn validate_session(&self, token: &str) -> bool;
}

/// Service managing password and session state.
pub struct AuthService {
    pub active: bool,
}

/// Crate-internal password helper.
pub(crate) fn verify_password_token(raw: &str) -> bool {
    !raw.is_empty()
}

/// Raw 64-bit secret representation.
#[repr(C)]
pub union RawSecret {
    pub bytes: [u8; 8],
    pub word: u64,
}

/// Default session timeout in seconds.
pub static SESSION_TIMEOUT_SECS: u64 = 900;

pub mod sub;

#[path = "custom_tests.rs"]
mod tests;

/// Opens a session from a destructured user/token pair.
pub fn open_session(pair: (&str, &str)) -> String {
    let (auth_user, auth_token) = pair;
    tracing::info!("opened {}", sub::compute_secret_digest(auth_user));
    let verified = Some(auth_token).expect("session token present");
    verified.to_owned()
}
"#;

const CORE_SUB_RS: &str = r"/// Computes a deterministic digest length for a secret label.
pub fn compute_secret_digest(input: &str) -> usize {
    input.len()
}
";

const CORE_CUSTOM_TESTS_RS: &str = r#"use super::verify_password_token;

pub fn smoke_check() -> bool {
    Some(verify_password_token("ok")).unwrap()
}
"#;

const APP_CARGO_TOML: &str = r#"[package]
name = "dogfood-app"
version = "0.1.0"
edition = "2024"

[dependencies]
dogfood_core = { path = "../core" }
"#;

const APP_MAIN_RS: &str = r#"use dogfood_core::{AuthService, Authenticator, SESSION_TIMEOUT_SECS, open_session};

mod broken;

struct AppAuth;

impl Authenticator for AppAuth {
    fn validate_session(&self, token: &str) -> bool {
        token.len() as u64 <= SESSION_TIMEOUT_SECS
    }
}

pub fn main() {
    let _service = AuthService { active: true };
    let auth = AppAuth;
    let tok = open_session(("bob", "secret-tok"));
    let _ok = auth.validate_session(&tok);
}
"#;

const APP_BROKEN_RS: &str = "pub fn unclosed_syntax_fixture( {\n";

fn write_workspace_fixture(root: &Path) -> TestResult {
    let files = [
        ("Cargo.toml", ROOT_CARGO_TOML),
        ("src/main.rs", ROOT_MAIN_RS),
        ("src/runner.rs", ROOT_RUNNER_RS),
        ("crates/core/Cargo.toml", CORE_CARGO_TOML),
        ("crates/core/src/lib.rs", CORE_LIB_RS),
        ("crates/core/src/sub.rs", CORE_SUB_RS),
        ("crates/core/src/custom_tests.rs", CORE_CUSTOM_TESTS_RS),
        ("crates/app/Cargo.toml", APP_CARGO_TOML),
        ("crates/app/src/main.rs", APP_MAIN_RS),
        ("crates/app/src/broken.rs", APP_BROKEN_RS),
    ];
    for (rel, body) in files {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).check("create fixture parent dir")?;
        }
        fs::write(&full, body).check("write fixture file")?;
    }
    Ok(())
}

fn run_sql(manager: &SourceSyncManager, path: &str, sql: &str) -> TestResult<Vec<Value>> {
    let req: QueryRequest =
        serde_json::from_value(json!({ "path": path, "sql": sql })).check("build QueryRequest")?;
    let resp = query(&Parameters(req), &NoopProgressSink, manager)
        .map_err(|e| e.message.into_owned())
        .check("execute query tool")?;
    Ok(resp.0.rows)
}

fn str_col<'a>(row: &'a Value, col: &str) -> TestResult<&'a str> {
    row.get(col)
        .and_then(Value::as_str)
        .check(&format!("column `{col}` is string in {row}"))
}

fn verify_enable_report_leads_and_structure(manager: &SourceSyncManager, path: &str) -> TestResult {
    let on_req: ProjectControlRequest =
        serde_json::from_value(json!({ "path": path, "action": "on" }))
            .check("build ProjectControlRequest")?;
    let on_resp = project_control(&Parameters(on_req), &NoopProgressSink, manager)
        .map_err(|e| e.message.into_owned())
        .check("enable dogfood workspace")?
        .0;
    ensure_eq(
        &on_resp.status,
        &ProjectControlStatus::Enabled,
        "project_control on enables the workspace",
    )?;
    let import = on_resp.import.check("initial import report present")?;
    ensure(
        import.added >= 10,
        format!(
            "initial import discovers all workspace files, got {}",
            import.added
        ),
    )?;

    let rep_req: ReportRequest =
        serde_json::from_value(json!({ "path": path })).check("build ReportRequest")?;
    let rep = report(&Parameters(rep_req), &NoopProgressSink, manager)
        .map_err(|e| e.message.into_owned())
        .check("run report")?
        .0;
    ensure(
        rep.languages
            .iter()
            .any(|lang| lang.language == "rust" && lang.file_count == 7 && lang.tested),
        format!(
            "report records 7 tested Rust files, got {:?}",
            rep.languages
        ),
    )?;
    ensure(
        rep.dependency_edges.inbound_edge_count >= 5,
        format!(
            "report counts resolved intra-workspace dependency edges, got {}",
            rep.dependency_edges.inbound_edge_count
        ),
    )?;

    let leads_req: LeadsRequest =
        serde_json::from_value(json!({ "path": path })).check("build LeadsRequest")?;
    let leads_resp = leads(&Parameters(leads_req), &NoopProgressSink, manager)
        .map_err(|e| e.message.into_owned())
        .check("run leads")?
        .0;
    for expected_mechanic in ["unwrap_calls", "expect_calls", "diagnostics"] {
        ensure(
            leads_resp
                .mechanic_counts
                .iter()
                .any(|m| m.mechanic == expected_mechanic && m.count >= 1),
            format!("leads counts >= 1 row for `{expected_mechanic}`"),
        )?;
    }

    let struct_req: StructureRequest = serde_json::from_value(json!({
        "path": path,
        "language": "rust",
        "query": include_str!("../examples/queries/rust_functions.scm"),
    }))
    .check("build StructureRequest")?;
    let struct_resp = structure(&Parameters(struct_req), &NoopProgressSink, manager)
        .map_err(|e| e.message.into_owned())
        .check("run rust_functions.scm structure query")?
        .0;
    for fn_name in [
        "open_session",
        "verify_password_token",
        "compute_secret_digest",
        "smoke_check",
    ] {
        ensure(
            struct_resp
                .matches
                .iter()
                .any(|m| m.capture_name == "fn_name" && m.text == fn_name),
            format!("rust_functions.scm captures `{fn_name}`"),
        )?;
    }
    Ok(())
}

fn verify_shipped_cookbook_sql_recipes(manager: &SourceSyncManager, path: &str) -> TestResult {
    verify_auth_panic_and_diag_recipes(manager, path)?;
    verify_blast_radius_recipes(manager, path)
}

fn verify_auth_panic_and_diag_recipes(manager: &SourceSyncManager, path: &str) -> TestResult {
    let auth_rows = run_sql(
        manager,
        path,
        include_str!("../examples/queries/auth_attack_surface.sql"),
    )?;
    for expected in [
        "Authenticator",
        "validate_session",
        "AuthService",
        "verify_password_token",
        "RawSecret",
        "SESSION_TIMEOUT_SECS",
        "open_session",
        "auth_user",
        "auth_token",
        "compute_secret_digest",
    ] {
        ensure(
            auth_rows
                .iter()
                .any(|r| r.get("name").and_then(Value::as_str) == Some(expected)),
            format!("auth_attack_surface.sql surfaces `{expected}`: {auth_rows:?}"),
        )?;
    }

    let panic_rows = run_sql(
        manager,
        path,
        include_str!("../examples/queries/find_unhandled_panics.sql"),
    )?;
    ensure(
        panic_rows
            .iter()
            .any(|r| r.get("path").and_then(Value::as_str) == Some("crates/core/src/lib.rs"))
            && panic_rows.iter().any(|r| {
                r.get("path").and_then(Value::as_str) == Some("crates/core/src/custom_tests.rs")
            }),
        format!("find_unhandled_panics.sql locates expect and unwrap call sites: {panic_rows:?}"),
    )?;

    let diag_rows = run_sql(
        manager,
        path,
        include_str!("../examples/queries/syntax_diagnostics.sql"),
    )?;
    ensure(
        diag_rows.iter().any(|r| {
            r.get("path").and_then(Value::as_str) == Some("crates/app/src/broken.rs")
                && r.get("severity").and_then(Value::as_str) == Some("error")
        }),
        format!("syntax_diagnostics.sql reports broken.rs syntax error: {diag_rows:?}"),
    )
}

fn verify_blast_radius_recipes(manager: &SourceSyncManager, path: &str) -> TestResult {
    let outbound = run_sql(
        manager,
        path,
        include_str!("../examples/queries/blast_radius_outbound.sql"),
    )?;
    ensure(
        outbound
            .iter()
            .any(|r| r.get("resolution_kind").and_then(Value::as_str) == Some("Resolved")),
        format!("blast_radius_outbound.sql counts Resolved edges: {outbound:?}"),
    )?;

    let inbound = run_sql(
        manager,
        path,
        include_str!("../examples/queries/blast_radius_inbound.sql"),
    )?;
    ensure(
        inbound.iter().any(|r| {
            r.get("path").and_then(Value::as_str) == Some("crates/core/src/lib.rs")
                && r.get("importing_files")
                    .and_then(Value::as_i64)
                    .is_some_and(|n| n >= 2)
        }),
        format!("blast_radius_inbound.sql counts cross-crate importers of core: {inbound:?}"),
    )?;

    let closure_verbatim = run_sql(
        manager,
        path,
        include_str!("../examples/queries/blast_radius_closure.sql"),
    )?;
    for transitive in [
        "src/runner.rs",
        "crates/core/src/lib.rs",
        "crates/core/src/sub.rs",
        "crates/core/src/custom_tests.rs",
    ] {
        ensure(
            closure_verbatim.iter().any(|r| {
                r.get("direction").and_then(Value::as_str) == Some("imports")
                    && r.get("path").and_then(Value::as_str) == Some(transitive)
            }),
            format!(
                "verbatim blast_radius_closure.sql reaches `{transitive}` from src/main.rs: {closure_verbatim:?}"
            ),
        )?;
    }

    let core_closure_sql = include_str!("../examples/queries/blast_radius_closure.sql")
        .replace("'src/main.rs'", "'crates/core/src/lib.rs'");
    let core_closure = run_sql(manager, path, &core_closure_sql)?;
    ensure(
        core_closure
            .iter()
            .any(|r| r.get("direction").and_then(Value::as_str) == Some("imports"))
            && core_closure.iter().any(|r| {
                r.get("direction").and_then(Value::as_str) == Some("imported_by")
                    && r.get("path").and_then(Value::as_str) == Some("crates/app/src/main.rs")
            }),
        format!("core closure reports both imports and imported_by: {core_closure:?}"),
    )
}

fn verify_rust_evidence_and_graph_contracts(manager: &SourceSyncManager, path: &str) -> TestResult {
    verify_rust_structure_evidence(manager, path)?;
    verify_rust_symbol_call_and_edge_contracts(manager, path)
}

fn verify_rust_structure_evidence(manager: &SourceSyncManager, path: &str) -> TestResult {
    let structures = run_sql(
        manager,
        path,
        "SELECT json_extract(e.payload, '$.name') AS name, \
                json_extract(e.payload, '$.kind') AS kind, \
                json_extract(e.payload, '$.visibility') AS visibility, \
                json_extract(e.payload, '$.doc_comment') AS doc_comment \
         FROM evidence e JOIN files f ON f.id = e.file_id \
         WHERE f.path = 'crates/core/src/lib.rs' AND e.kind = 'Structure'",
    )?;
    let check_struct = |name: &str, kind: &str, vis: Option<&str>, doc_sub: &str| -> TestResult {
        let row = structures
            .iter()
            .find(|r| r.get("name").and_then(Value::as_str) == Some(name))
            .check(&format!("Structure row for `{name}`"))?;
        ensure_eq(&str_col(row, "kind")?, &kind, format!("`{name}` kind"))?;
        ensure_eq(
            &row.get("visibility").and_then(Value::as_str),
            &vis,
            format!("`{name}` visibility"),
        )?;
        let doc = str_col(row, "doc_comment")?;
        ensure(
            doc.contains(doc_sub) && !doc.contains("inner module docs"),
            format!("`{name}` doc_comment has outer doc without //! leakage: {doc:?}"),
        )
    };
    check_struct(
        "Authenticator",
        "Trait",
        Some("pub"),
        "Authenticates incoming requests",
    )?;
    check_struct(
        "validate_session",
        "Method",
        None,
        "Validates an opaque session token",
    )?;
    check_struct(
        "verify_password_token",
        "Function",
        Some("pub(crate)"),
        "Crate-internal password helper",
    )?;
    check_struct("RawSecret", "Union", Some("pub"), "Raw 64-bit secret")
}

fn verify_rust_symbol_call_and_edge_contracts(
    manager: &SourceSyncManager,
    path: &str,
) -> TestResult {
    let symbols = run_sql(
        manager,
        path,
        "SELECT json_extract(e.payload, '$.name') AS name, \
                json_extract(e.payload, '$.kind') AS kind, \
                json_extract(e.payload, '$.visibility') AS visibility \
         FROM evidence e JOIN files f ON f.id = e.file_id \
         WHERE f.path = 'crates/core/src/lib.rs' AND e.kind = 'Symbol'",
    )?;
    ensure(
        symbols.iter().any(|r| {
            r.get("name").and_then(Value::as_str) == Some("SESSION_TIMEOUT_SECS")
                && r.get("kind").and_then(Value::as_str) == Some("Constant")
                && r.get("visibility").and_then(Value::as_str) == Some("pub")
        }),
        format!("static SESSION_TIMEOUT_SECS indexed as pub Constant: {symbols:?}"),
    )?;
    for destructured in ["auth_user", "auth_token"] {
        ensure(
            symbols
                .iter()
                .any(|r| r.get("name").and_then(Value::as_str) == Some(destructured)),
            format!("destructured let binding `{destructured}` indexed: {symbols:?}"),
        )?;
    }

    let calls = run_sql(
        manager,
        path,
        "SELECT json_extract(e.payload, '$.callee_name') AS callee_name, \
                json_extract(e.payload, '$.caller') AS caller, \
                json_extract(e.payload, '$.resolution') AS resolution \
         FROM evidence e JOIN files f ON f.id = e.file_id \
         WHERE f.path = 'crates/core/src/lib.rs' AND e.kind = 'Call'",
    )?;
    ensure(
        calls.iter().any(|r| {
            r.get("callee_name").and_then(Value::as_str) == Some("compute_secret_digest")
                && r.get("caller").and_then(Value::as_str) == Some("open_session")
        }),
        format!("nested call inside tracing::info!(...) macro extracted: {calls:?}"),
    )?;

    let edges = run_sql(
        manager,
        path,
        "SELECT src.path AS from_path, dst.path AS to_path, e.resolution_kind \
         FROM dependency_edges e \
         JOIN files src ON src.id = e.from_file_id \
         JOIN files dst ON dst.id = e.to_file_id \
         WHERE e.resolution_kind = 'Resolved'",
    )?;
    for (from_path, to_path) in [
        ("crates/app/src/main.rs", "crates/core/src/lib.rs"),
        ("crates/core/src/lib.rs", "crates/core/src/sub.rs"),
        ("crates/core/src/lib.rs", "crates/core/src/custom_tests.rs"),
        ("crates/core/src/custom_tests.rs", "crates/core/src/lib.rs"),
    ] {
        ensure(
            edges.iter().any(|r| {
                r.get("from_path").and_then(Value::as_str) == Some(from_path)
                    && r.get("to_path").and_then(Value::as_str) == Some(to_path)
            }),
            format!("expected Resolved edge `{from_path}` -> `{to_path}` in {edges:?}"),
        )?;
    }
    Ok(())
}

#[test]
fn multi_crate_rust_workspace_executes_every_cookbook_recipe_with_accurate_evidence() -> TestResult
{
    let workspace = tempfile::tempdir().check("create dogfood workspace tempdir")?;
    write_workspace_fixture(workspace.path())?;
    let path = workspace.path().to_string_lossy().replace('\\', "/");
    let manager = SourceSyncManager::new();

    verify_enable_report_leads_and_structure(&manager, &path)?;
    verify_shipped_cookbook_sql_recipes(&manager, &path)?;
    verify_rust_evidence_and_graph_contracts(&manager, &path)
}
