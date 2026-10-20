# Wave 4 — R8 public API contract

Main before wave 4: `d669848`; wave 4 commits `3dc9e42`..`976dc2e`, doc-link
fix `27ae582`, surface `cf1475e`. Date: 2026-10-19.

## Scope

The strict audit's 258 R8 production findings: 241 current missing-doc sites
plus six external-use items (the other audit entries had already been
documented, removed, or moved by waves 1–3).

## Result

- Missing-doc sites: **241 → 0** (`RUSTFLAGS='-W missing_docs' cargo check
  --lib --bins`).
- `cargo doc --no-deps --lib`: **0 warnings** after the final 10 intra-doc link
  fixes.
- External-use decisions:
  - `cli::USAGE` → `pub(crate)` (no external caller; `cli::usage()` unchanged).
  - `SlugAuditServer::with_manager` → `pub(crate)` (only internal callers).
  - `LineIndex::line` → `pub(crate)` + `#[cfg(test)]` (only the span test calls
    it; production uses `column`/`position`).
  - `ErrorCode::as_str` → `pub(crate)`.
  - `WatchScopeLimit` → `pub(crate)`, removed from the watch re-export;
    `WatchScope::limit` crate-private.
  - `ErrorCode::ALL` → `#[cfg(test)] pub(crate)`; the ARCHITECTURE sentence
    that named it was updated to the test-local enumeration.
- Public doc comments on MCP types become JSON-Schema descriptions; the final
  `.planning/archive/mcp_surface.json` was re-recorded from the integrated tree
  (8 tools, 8 resources).
- `src/tools/health.rs`'s stale source-limit annotation comment was removed.

## Release assets

`cargo-about 0.9.2` (the release workflow's version) rendered the notices
through the exact workflow command:

- **6,558 lines / 340,298 bytes**, exit 0, no warnings.
- Fixed an HTML-escaping defect: license names now use `{{{name}}}` in the
  template (no `&quot;` entities).
- Removed the machine-specific `source_path` block, so no `/root/.cargo/...`,
  `/home/runner/...`, or registry paths ship to consumers.
- 160 unique (crate, version) pairs match the release graph; root crate absent;
  184 license assignments; 74 license-text sections.
- `ring` 0.17.14's ISC text is extracted from source-file headers and includes
  trailing code fragments; this is upstream cargo-about behavior (ring's
  LICENSE points only to `LICENSE-other-bits`), recorded as a tool limitation,
  not a template change.

## Validation

Full gate on the fixed tree (`4b765e1`):

- fmt, clippy, deny, machete, source limits, duplicate checks, producer
  coverage, governance, version: pass.
- **1,176 tests passed, 0 failed, 1 ignored.**
- Coverage **93.27%** (11,661 / 12,503; gate 83%).
- `missing_docs`: **0**; `cargo doc --no-deps --lib`: **0 warnings**.
- Docs drift: **PASS**.
- MCP surface: **PASS** (re-recorded).
- Performance: pass, 0.97x–1.09x against the 20% threshold.

Two gate-discovered fixes were required: the `EvidenceKind` schema-derivation
test now accepts the documented `oneOf` shape, and `cli.rs`/`revision.rs` were
kept under 200 code lines by moving `usage`/`USAGE` and
`invalidate_stale_findings` into `cli/usage.rs` and `sync/revision_support.rs`.
The mutation scope was widened to six files for `revision_support.rs`.
