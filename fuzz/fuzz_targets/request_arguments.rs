#![no_main]
//! Target 3 — the JSON-RPC request boundary this repository actually owns.
//!
//! **There is no frame parser here to fuzz.** `src/main.rs` hands stdio
//! straight to the dependency:
//!
//! ```ignore
//! SlugAuditServer::new().serve(stdio()).await?.waiting().await?
//! ```
//!
//! `rmcp 3.1.4` owns framing, the newline-delimited JSON reader, request-id
//! correlation, and method dispatch; the repository contributes no byte-level
//! parser anywhere on that path (the only `BufRead::read_line` calls in `src/`
//! are `menu.rs`'s and `install.rs`'s interactive prompts, which never run
//! under `serve`). Fuzzing "JSON-RPC frame parsing" as written would therefore
//! be fuzzing rmcp, which this target deliberately does not do — a defect found
//! in a dependency's parser belongs upstream, and reporting it as this
//! repository's would be a false finding.
//!
//! What the repository does own on that path is the decode of
//! `params.arguments` into the nine tool request types: the `serde`-derived
//! `Deserialize` impls in `src/tools/*.rs`, with the contract they carry
//! (`#[serde(default)]` on optional fields, `Option` fields that must tolerate
//! absence, `rename_all = "snake_case"` sets, `ProjectControlAction`'s
//! `"on"`/`"off"` renames, `usize` fields, and the closed variant sets of
//! `LeadMode` and `MechanicAction`). rmcp's tool router calls exactly this decode, so a panic or a hang
//! here is a panic or a hang in every tool call. That is the boundary fuzzed
//! below.
//!
//! Honest value statement: the impls are derived, so the realistic yield is
//! low — it proves the declared contract cannot be made to panic, abort, or
//! recurse without bound by a hostile `arguments` object, and nothing more. It
//! is not a substitute for the dependency's parser and does not claim to be.
//!
//! Input layout: byte 0 selects which request type the rest is decoded as.

use libfuzzer_sys::fuzz_target;
use slugaudit_mcp_rust::tools::{
    FindingReadRequest, FindingRequest, HealthRequest, LeadsRequest, MechanicRequest,
    ProjectControlRequest, QueryRequest, ReportRequest, StructureRequest,
};

/// The nine tool request types, i.e. the `arguments` object of every
/// `tools/call` the server accepts.
const KINDS: usize = 9;

/// `query`'s and `structure`'s instance limits are the only per-field bounds
/// the decode itself does not apply, and neither is a decode concern: a
/// 200 KB `arguments` object decodes fine and is then refused by the tool. Cap
/// the input so iterations stay cheap without hiding the decode path.
const MAX_ARGUMENT_BYTES: usize = 128 * 1024;

fuzz_target!(|data: &[u8]| {
    let Some((selector, body)) = data.split_first() else {
        return;
    };
    if body.len() > MAX_ARGUMENT_BYTES {
        return;
    }
    decode(*selector as usize % KINDS, body);
});

/// Decodes `body` as one of the request types. The result is dropped: a
/// `tools/call` carrying arguments that do not fit its schema is an
/// `invalid_params` response, not a defect, so only the decoder's own
/// behaviour (panic, abort, unbounded recursion) is under test.
fn decode(kind: usize, body: &[u8]) {
    macro_rules! decode_as {
        ($request:ty) => {{
            let _ = serde_json::from_slice::<$request>(body);
        }};
    }
    match kind {
        0 => decode_as!(QueryRequest),
        1 => decode_as!(StructureRequest),
        2 => decode_as!(LeadsRequest),
        3 => decode_as!(FindingRequest),
        4 => decode_as!(FindingReadRequest),
        5 => decode_as!(ReportRequest),
        6 => decode_as!(HealthRequest),
        7 => decode_as!(MechanicRequest),
        _ => decode_as!(ProjectControlRequest),
    }
}
