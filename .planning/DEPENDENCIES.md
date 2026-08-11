# Dependency policy and inventory

Status: current as of 2026-08-10 (pinned compiler `1.97.1`, edition `2024`).

This document records the dependency policy required by Task 11.2 and the
current inventory. The machine-enforced policy lives in `deny.toml`
(`cargo deny check advisories bans sources licenses`); this file records the
rationale, the native/FFI boundary, the transitive-unsafe inventory, and the
update procedure.

## Policy

1. **Advisories**: known vulnerabilities are denied via RustSec
   advisory-db (`cargo audit` and `cargo deny check advisories`). There is no
   severity threshold carve-out — any advisory against a locked version
   blocks release until the dependency is updated or the advisory is
   documented as not applicable (which requires a dated decision-log entry).
2. **Licenses**: only the allow-list in `deny.toml` is permitted
   (`Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `CC0-1.0`,
   `CDLA-Permissive-2.0`, `ISC`, `MIT`, `PolyForm-Noncommercial-1.0.0`,
   `MPL-2.0`, `Unicode-3.0`, `Zlib`). Anything else requires a reviewed
   exception recorded here and in `deny.toml`'s `exceptions` list before it
   may be added.
3. **Sources**: crates must come from `crates.io`. Unknown registries and
   git dependencies are denied (`deny.toml` `[sources]`). Vendored or
   forked dependencies require a dated decision-log entry.
4. **Unsafe boundary**: `#![forbid(unsafe_code)]` is compiler-enforced for
   SlugAudit source. Transitive unsafe in dependencies is expected where FFI
   is required (tree-sitter, bundled SQLite, tokio) and is reviewed by hand
   when a dependency is added (see "Native/FFI and transitive unsafe" below).
   `cargo geiger --all-features` is run as an inventory, not a gate — its
   per-crate counts are reviewed, and any *new* dependency that introduces
   unsafe must be named in this document at the time it is added.
5. **Duplicate versions**: `cargo deny check bans` warns on
   multiple-versions. Before bumping a dependency, check whether the bump
   collapses or introduces a duplicate and record the outcome.
6. **Updates**: dependencies change only under the pinned compiler
   (`1.97.1`) and only with the full validation gate run
   (`.planning/RELEASE_CHECKLIST.md`). Lockfile and toolchain updates are
   separate, deliberate changes — never folded into feature work.

## Direct dependencies (2026-08-10)

Versions are the locked versions in `Cargo.lock` as of this writing.

| Crate | Locked | Purpose | Native/FFI |
|---|---|---|---|
| `blake3` | 1.8.6 | Content hashing of raw file bytes (manifest, findings invalidation) | yes (SIMD; Rust impl with C fallbacks) |
| `ignore` | 0.4.31 | Ignore-aware file discovery (`.gitignore`-compatible) | no |
| `notify` | 8.2.0 | Per-project filesystem watcher for incremental sync | yes (inotify/kqueue/FSEvents/Win32 backends) |
| `rmcp` | 2.2.0 | MCP server SDK: stdio transport, tool schema, protocol frames, progress notifications | no (async over tokio) |
| `rusqlite` | 0.37.0 | SQLite access with `bundled` SQLite and `hooks` features | yes (bundled SQLite C library) |
| `schemars` | 1.2.2 | JSON-schema derivation for MCP tool input schemas | no |
| `serde` / `serde_json` | 1.0.229 / 1.0.151 | Typed request/response serialization and JSON-RPC payloads | no |
| `sha2` | 0.10.9 | Secondary hashing where a second digest is useful | no (Rust) |
| `thiserror` | 2.0.19 | Typed error enums across module boundaries | no |
| `tokio` | 1.53.1 | Async runtime: stdio transport, watcher tasks, spawn_blocking tool calls | yes (epoll/kqueue/io_uring) |
| `tracing` / `tracing-subscriber` | 0.1.44 / 0.3.23 | Stderr-only structured logs and per-call spans | no |
| `tree-sitter` | 0.26.11 | Parse-tree API used by the language pack | yes (tree-sitter C library) |
| `tree-sitter-language-pack` | 1.13.7 (pinned `=1.13.7`) | The 306-parser language pack: detection, on-demand download/cache, generic `process()`, `get_parser()`, aliases, ABI handling | yes (bundled C grammars; build-time parser-sources download) |
| `which` | 7.0.3 | Locating external tooling (e.g. language-pack parser sources fetch helpers) | no |

### Dev dependencies

| Crate | Locked | Purpose |
|---|---|---|
| `proptest` | 1.11.0 | Property tests for paths, spans, evidence limits, resolution round-trips |
| `tempfile` | 3.27.0 | Fixture databases, temp project roots, watcher-scenario scratch dirs |
| `criterion` | 0.5.1 | Benchmark harness for `benches/` (Task 9.2); `harness = false` bench targets only |
| `temp-env` | 0.3.6 | Safe env-var scoping for `install`/`connect` tests (`set_var` is unsafe under edition 2024 + `forbid(unsafe_code)`); Apache-2.0/MIT |

`criterion` is a dev-dependency only, so the release binary is unaffected.
Benchmark builds are separate targets and are never part of the correctness
gates (`cargo test --all-targets` compiles them but does not run them);
results and the run protocol live in `.planning/PERFORMANCE.md`. Default
features are used (no `html_reports`, so no plotters in the graph).

## Native/FFI and transitive unsafe

SlugAudit source contains no unsafe (compiler-enforced). The following
direct dependencies perform FFI/syscalls and internally contain unsafe,
which is expected and reviewed:

- `tree-sitter` / `tree-sitter-language-pack` — C grammar library and bundled
  parsers; required for parsing. The pack's `build.rs` downloads a
  parser-sources tarball at build time on a cold cache (CI caches this via
  `Swatinem/rust-cache`); the runtime parser-pack cache is a separate,
  surfaced concern (see `OBSERVABILITY.md`).
- `rusqlite` (bundled SQLite) — C database engine; the only persistence
  backend.
- `tokio` — async runtime with OS-level I/O.
- `notify` — OS filesystem-watcher backends.
- `rmcp`'s transitive `ring`/`rustls` — TLS for future stream transports
  (not exercised by the stdio-only product today, but present in the graph).

Full transitive inventory is available via `cargo geiger --all-features`;
the CI step treats it as a review inventory rather than a pass/fail gate
(see the comment in `.github/workflows/quality.yml`).

## Verification

```bash
cargo audit                       # RustSec advisory-db against Cargo.lock
cargo deny check advisories bans sources licenses
cargo geiger --all-features       # inventory of transitive unsafe
cargo tree --duplicates           # duplicate-version review
```

All of the above pass as of 2026-08-10 (`cargo audit` reports zero known
vulnerabilities; `cargo deny` passes with the allow-list above; the lockfile
contains 257 packages across the resolved graph).
