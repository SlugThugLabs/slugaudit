#![no_main]
//! Target 1 — Tree-sitter query strings fed to `structure`.
//!
//! `structure` is the most interesting surface in the threat model
//! (`AGENTS.md`, *Security and trust model*): the agent supplies a query, and
//! the server compiles and executes it against its own indexed sources. The
//! narrowest public entry point that takes a query string is therefore
//! [`structure`] itself — the handlers in `src/server.rs` only add progress
//! plumbing and a blocking-pool hop, and the query compilation
//! (`Query::new`), the capture cursor, the match-count/time budget, and
//! `node_to_match`'s span arithmetic are all downstream of it and all private.
//!
//! What this exercises per iteration: `structure`'s own validation (empty,
//! oversized), `tree_sitter_language_pack::get_language`, `Query::new` on
//! hostile text, a real parse of indexed content, the capture cursor with its
//! progress-callback deadline, and node → match conversion including
//! `model::span::char_column`, which slices the source at a node's byte offset
//! and used to panic on an offset that was not a char boundary.
//!
//! Input layout: byte 0 is a selector (which fixture file, single- vs
//! multi-file mode, `full_text`); the rest is the query text. The project
//! fixture is built once per process (see `support.rs`) so an iteration
//! measures the query path rather than a re-sync.
//!
//! **Known non-finding.** Tree-sitter 0.26.13 leaks 8 bytes when a query names
//! a real node kind with a field that kind does not define — the minimized
//! input is `(identifier zzz: (identifier) @a)`. The allocation is made and
//! abandoned inside the dependency's C query compiler (`ts_query_new` →
//! `ts_query__parse_pattern` → `_array__reserve`), `Query::new` returns `Err`,
//! and nothing in this crate retains anything, so there is no `src/` fix. It
//! does not reproduce for a valid query, for `((((`, or for an unknown node
//! kind, so it is the field-name error path specifically; a probe that called
//! only `tree_sitter_language_pack::get_language` + `tree_sitter::Query::new`
//! and no SlugAudit code reproduced it, which is what rules this crate out as
//! the owner. `fuzz/lsan-suppressions.txt` suppresses exactly that allocation
//! (LeakSanitizer prints `count 1, bytes 8` each run), and the scheduled
//! workflow supplies the symbolizer the suppressions need. Delete the
//! suppression when the pinned tree-sitter stops leaking on that path.

use libfuzzer_sys::fuzz_target;
use rmcp::handler::server::wrapper::{Json, Parameters};
use slugaudit_mcp_rust::parse::detect_language;
use slugaudit_mcp_rust::progress::NoopProgressSink;
use slugaudit_mcp_rust::tools::{StructureRequest, structure};

#[path = "support.rs"]
mod support;

/// `ResourceLimits::default().max_structure_query_bytes` is 32_000; anything
/// past it is refused before compilation, so a longer input is a wasted
/// iteration rather than new coverage.
const MAX_QUERY_BYTES: usize = 32_000;

/// Selector bit: run project-wide with `language` instead of a single `file`.
const MULTI_FILE: u8 = 0x20;
/// Selector bit: ask for full node text rather than the lean one-line snippet.
const FULL_TEXT: u8 = 0x40;

fuzz_target!(|data: &[u8]| {
    let Some((selector, body)) = data.split_first() else {
        return;
    };
    if body.len() > MAX_QUERY_BYTES {
        return;
    }
    let fixture = &*support::FIXTURE;
    let (file, _) = support::FILES[*selector as usize % support::FILES.len()];

    // `language` mode has no `file`, so the grammar has to be named; reuse the
    // one the pack detects for the selected fixture path instead of hard-coding
    // pack language ids in the harness.
    let (file_field, language_field) = if selector & MULTI_FILE == 0 {
        (Some(file.to_owned()), None)
    } else {
        let language = detect_language(file).unwrap_or_else(|| "rust".to_owned());
        (None, Some(language))
    };

    let request = Parameters(StructureRequest {
        path: fixture.path(),
        file: file_field,
        language: language_field,
        pattern: None,
        query: String::from_utf8_lossy(body).into_owned(),
        full_text: Some(selector & FULL_TEXT != 0),
        ..Default::default()
    });

    // Errors are the expected outcome for most hostile queries (a malformed
    // S-expression, a capture-free query, or the execution-time budget) and
    // are not the target's subject. A success is checked against the response
    // contract: an AST node's span is ordered, and nothing here may return an
    // inverted one.
    if let Ok(Json(response)) = structure(&request, &NoopProgressSink, &fixture.manager) {
        for m in &response.matches {
            assert!(
                m.start_byte <= m.end_byte,
                "inverted byte span {:?}..{:?} for capture {:?} in {}",
                m.start_byte,
                m.end_byte,
                m.capture_name,
                m.file
            );
            assert!(
                m.start_line <= m.end_line,
                "inverted line span {:?}..{:?} for capture {:?} in {}",
                m.start_line,
                m.end_line,
                m.capture_name,
                m.file
            );
        }
    }
});
