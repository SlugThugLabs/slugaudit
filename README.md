# 🐌 SlugAudit

SlugAudit is a stdio [MCP](https://modelcontextprotocol.io) server. It indexes a
project into a disposable SQLite / Tree-sitter evidence index and answers the calling
agent's queries, instead of making the agent read files to reconstruct the same facts.

## What it is

SlugAudit is built for AI agents, not for humans. It is an index of a project's code —
parsed structure, symbols, imports, call sites, comments, and diagnostics — that an
agent queries over MCP.

- **Evidence, not judgment.** SlugAudit performs no risk detection, assigns no
  severity, ranks nothing, and draws no conclusions. Every judgment — what is buggy,
  how severe, what to fix — belongs to the calling agent. Sorting out which facts
  matter is the agent's job, not the tool's.
- **Facts with coordinates.** Evidence rows carry a file path and a line span, so an
  agent can act on them without re-reading the tree.
- **Freshness without re-verification.** The index is re-synchronized before every
  tool call, so an answer describes the files on disk (not unsaved editor buffers) and
  the agent never has to double-check whether its view is current.
- **Caps are reported in the response.** A paged or capped result carries `truncated`
  and, when more rows exist, the `next_offset` to continue from; `leads` rows flag
  detail text that was clipped (`detail_clipped`).

For each enabled project, SlugAudit owns exactly one derived-data directory:

```text
<project>/.planning/slugaudit/project.db
```

The surrounding `.planning/` directory belongs to the project, is indexed normally,
and SlugAudit's own `.planning/slugaudit/` is excluded from discovery. The database is
disposable: delete it and the next tool call rebuilds it from source. No fact lives
only in the index.

## Install

Prebuilt binaries are published for Linux x86_64. On any other platform, build from
source with Rust 1.97.1 (edition 2024) — the toolchain version is pinned in
`rust-toolchain.toml`.

```bash
# Installer script: downloads the Linux x86_64 release (SHA256-verified),
# falls back to `cargo install --git`, and builds from a local checkout when
# run inside this repository.
curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash

# From a source checkout
cargo install --path .

# Interactive setup: install, connect an agent, disconnect, instructions for
# any other MCP client, or run the server
slugaudit menu
```

The installer script and `slugaudit install` both place the binary at
`~/.slugthug/slugaudit/slugaudit`, a stable path that survives rebuilds — which is
what `slugaudit connect` registers with your agent. `slugaudit update` downloads a
newer release and atomically replaces that binary.

**Grammars are fetched on first use.** SlugAudit does not bundle Tree-sitter grammars.
The first time a language is parsed, the pinned `tree-sitter-language-pack` downloads a
compressed bundle of grammars (about 22 MB) from its GitHub release
(`github.com/xberg-io/tree-sitter-language-pack`), verifies it against the sha256 in the
release manifest, and caches it at `~/.cache/tree-sitter-language-pack/v1.18.0/libs`
(the platform cache directory; `TREE_SITTER_LANGUAGE_PACK_CACHE_DIR` overrides its
base). Later runs load from that cache without touching the network. If the fetch fails
(for example, offline on a first run), the file's content is still stored and
searchable, its `files.parser_availability` is `LoadFailed`, and it is retried after a
10-minute cool-down (or on the next server start) instead of being left unparsed. The
first failed fetch can block one call for up to about 60 seconds (the pack's HTTP
timeout, which it does not let us change); during the cool-down no further fetch is
attempted and no new revision is published for those files. The grammar fetch and `slugaudit update` are
the only outbound network use; the server itself runs no network listener.

Every release ships the material to check what you downloaded: `SHA256SUMS` for
the binary, `LICENSE`, a CycloneDX SBOM, a third-party notices file, and a signed
build-provenance attestation. [docs/RELEASING.md](docs/RELEASING.md) has the
exact commands and states what each one proves — including why a `SHA256SUMS`
published alongside the artifact is integrity rather than authenticity, and why
the attestation is what closes that gap.

## Connect an agent

`slugaudit connect` writes a `slugaudit` MCP server entry into an agent's or editor's
own configuration. With no argument it lists the agents detected on the machine to
pick from; with a name it configures that target directly.

```bash
slugaudit connect claude     # Claude Code (~/.claude.json)
slugaudit disconnect claude  # alias: slugaudit remove claude
```

23 targets in this revision (ids are the `connect` argument; the display name is in
parentheses where the id is not self-evident):

`agy` (Antigravity), `amp`, `bob`, `claude` (Claude Code), `claude-desktop`, `codex`,
`copilot` (GitHub Copilot), `crush`, `cursor`, `droid` (Droid / Factory), `gemini`
(Gemini CLI), `goose`, `grok`, `hermes`, `kimi` (Kimi Code), `omp` (Oh My Pi),
`opencode`, `openhands`, `pi`, `q` (Amazon Q Developer), `warp`, `windsurf`, `zed`.

`claude-code` is accepted as an alias for `claude`, `antigravity` for `agy`,
`github-copilot` for `copilot`, and `oh-my-pi` for `omp`. The registry is
`src/connect_agents.rs`; each target's config path and scope is listed in
[docs/CLIENTS.md](docs/CLIENTS.md).

For a client that is not in that list, `slugaudit menu` (option 4) prints a JSON
snippet you can paste into any MCP client's config.

## Tools

The server exposes 8 MCP tools, registered in `src/server.rs`:

| Tool | What it returns |
| :--- | :--- |
| `report` | Project snapshot: file and language counts, parser failures, evidence-kind counts, open findings, inbound/outbound dependency-edge totals. |
| `leads` | Unranked mechanical facts: call sites by callee (`unwrap`, `expect`, `panic`, `todo`, `unimplemented`, `assert`), diagnostics, `TODO`/`FIXME`/`HACK` markers, credential-shaped files. Counts by default; paged file/line rows for one `mechanic` on request. Callee-based mechanics read Rust, Python, JavaScript, TypeScript, TSX, Go, Java and C calls; each count's `covers` lists the languages it read. |
| `mechanic` | Registers (`add`), removes or lists the auditor's own mechanics: a Tree-sitter query with one capture over one language, counted by `leads` beside the built-ins. Session entries end with the server; `persist: true` writes them to `.planning/slugaudit/config.json`. Built-ins can never be removed or overridden. |
| `query` | Read-only SQL over the index (`files`, `evidence`, `dependency_edges`), including full file text via `files.content`. |
| `structure` | Tree-sitter AST queries in one file or across a language, returning matched nodes with line spans and snippets. |
| `finding` | Persists a conclusion the agent has reviewed, bound to the file's current content hash — it goes stale when that file changes. The judgment is the agent's; SlugAudit never generates it. |
| `finding_read` | Findings written by the current session only, never another session's conclusions. |
| `project_control` | Enable (`on`) or disable (`off`) a project, optionally setting its profile; `off` purges the database. |
| `health` | Operational snapshot: watcher health, unreconciled event counts, current revision and file count, parser-pack version, tool-call counters. |

Every tool advertises a `title`, MCP `annotations` (`readOnlyHint` for the read-only
tools; `destructiveHint` only on `project_control`, whose `off` action purges the
database), and an `outputSchema` derived from its response type. Tool schemas and usage
guidance are served to the agent at runtime by the server itself (`INSTRUCTIONS` in
`src/server.rs`); the per-kind `evidence.payload` keys live in
`slugaudit://queries/evidence_payload_keys.md` instead of in the briefing.

## Resources

Eight pull-based MCP resources, read with `resources/read`. They cost an agent nothing
in context unless it asks for one. The query resources are also in the repository under
[`examples/queries/`](examples/queries/), and are compiled into the binary from those
files:

- `slugaudit://queries/README.md` — SQL and Tree-sitter recipes ([`examples/queries/README.md`](examples/queries/README.md))
- `slugaudit://queries/evidence_payload_keys.md` — every `evidence.payload` key, its JSON type, and the kind it belongs to
- `slugaudit://queries/find_unhandled_panics.sql` — `unwrap` / `expect` / `panic` call sites
- `slugaudit://queries/auth_attack_surface.sql` — functions, methods, and calls touching auth, tokens, or sessions
- `slugaudit://queries/syntax_diagnostics.sql` — syntax-error and parser diagnostics per file
- `slugaudit://queries/rust_functions.scm` — Tree-sitter pattern for Rust functions
- `slugaudit://queries/blast_radius.sql` — per-file inbound/outbound `dependency_edges` counts and the transitive closure
- `slugaudit://schema.sql` — the index schema (from `src/store/schema.sql`)

The `tools/list` and `resources/list` contract — names, titles, descriptions, input and
output schemas, annotations, and this catalogue — is pinned by `check_mcp_surface`
against `.planning/archive/mcp_surface.json`.

## Configuration

Three resource profiles scale the per-file, import, query, and sync budgets
(`src/model/profile.rs`):

- `adaptive` (default) — sizes limits from detected host memory.
- `audit` — large caps and long budgets for large repositories.
- `lean` — small caps for constrained environments.

Set the profile in a project as `.planning/slugaudit/config.json`:

```json
{ "profile": "audit" }
```

…or pass it to the `project_control` tool, or set `SLUGAUDIT_PROFILE=audit`. Individual
limits can be overridden with environment variables named `SLUGAUDIT_<FIELD>` (for
example `SLUGAUDIT_MAX_FILE_BYTES`, `SLUGAUDIT_MAX_SYNC_WALL_CLOCK_SECS`); each field
and its variable are documented in `src/model/limits.rs`. `query` and `leads` return
64 KiB pages by default (`SLUGAUDIT_DEFAULT_PAGE_BYTES`, or `default_page_bytes` in
`config.json`); a request's `page_bytes` raises it up to `SLUGAUDIT_MAX_QUERY_RESPONSE_BYTES`. Files larger than
`SLUGAUDIT_MAX_PARSE_BYTES` (2 MiB by default, or `max_parse_bytes` in `config.json`) are stored and
searchable but not parsed; `leads` reports them in `files_not_parsed` and `report` in its completeness
block. A file whose parse takes longer than `SLUGAUDIT_MAX_PARSE_WALL_CLOCK_MS` (5000 by default, or
`max_parse_wall_clock_ms` in `config.json`) is stored without structure and counted the same way; the
sync goes on, and the file is parsed again only when it changes. An unset or unparseable override is ignored, leaving the default in place.

Logging goes to `stderr`; `stdout` carries JSON-RPC only. Set
`SLUGAUDIT_LOG_FORMAT=json` for one-line JSON log events.

## Documentation

- [AGENTS.md](AGENTS.md) — why this exists, the founding metric, non-negotiables, verified conventions
- [.planning/ARCHITECTURE.md](.planning/ARCHITECTURE.md) — design, invariants, data flow, module map
- [.planning/PLAN.md](.planning/PLAN.md) — the active plan
- [.planning/archive/DECISIONS.md](.planning/archive/DECISIONS.md) — decision log (append-only history)
- [docs/CLIENTS.md](docs/CLIENTS.md) — every `connect` target, its config path, and its scope
- [docs/RELEASING.md](docs/RELEASING.md) — what a release publishes, and how to verify it (checksums, SBOM, attestation)
- [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) — operator runbook

## Development

Rust 1.97.1 (edition 2024). The same gates CI runs:

```bash
cargo fmt --all -- --check
cargo run --quiet --bin check_source_limits --locked
cargo run --quiet --bin check_no_duplicates --locked
cargo run --quiet --bin check_docs_drift --locked
cargo run --quiet --bin check_producer_coverage --locked
cargo run --quiet --bin check_mcp_surface --locked
cargo run --quiet --bin check_governance --locked
cargo run --quiet --bin check_version_consistency --locked
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --bins --tests --all-features --locked
cargo run --quiet --bin check_coverage --locked
```

CI also runs `cargo-deny` (advisories, bans, sources, licences — its `advisories`
check is the RustSec advisory database, with nothing suppressed by `deny.toml`)
and scoped mutation testing. Every third-party action in `.github/workflows/` is
pinned to a full commit SHA, and [`.github/dependabot.yml`](.github/dependabot.yml)
proposes the next pin weekly for both `cargo` and `github-actions`.
Two workflows cover what a push cannot:
[`.github/workflows/advisories.yml`](.github/workflows/advisories.yml) re-runs the
same `deny.toml` policy weekly, so an advisory published against an unchanged
`Cargo.lock` is visible without waiting for a push, and
[`.github/workflows/supply-chain.yml`](.github/workflows/supply-chain.yml) gates
each pull request and each push to `main` — dependency review of the diff, an
OSV.dev scan of `Cargo.lock`, and a secret scan of the working tree, which skips
the runtime state directory `.planning/slugaudit/` exactly as GitHub's own secret
scanning does via
[`.github/secret_scanning.yml`](.github/secret_scanning.yml).
`check_mcp_surface` fails when `tools/list` or `resources/list`
drifts from the committed `.planning/archive/mcp_surface.json`; a deliberate surface
change is recorded with `cargo run --bin check_mcp_surface -- --record`.

`check_performance` compares the criterion benches against the committed baseline
at `.planning/archive/perf_baseline.json`, and it is **local-only: CI does not run
it**. The reason is the baseline itself — it records the machine that measured it,
and criterion medians on a shared, unpinned GitHub runner differ from that machine
for reasons unrelated to the commit, so the 20 percent allowance would measure the
runner rather than the change. Its value is a same-machine before/after
comparison, so run it on the machine you are changing.
`check_coverage` gates line coverage at 83% by default — the threshold is
calibrated against measurement, not an aspirational target.

### Unsafe code policy

`unsafe_code = "forbid"` is set crate-wide in `Cargo.toml`, so this crate's own source
contains no `unsafe` blocks. Transitive unsafe inside dependencies that perform FFI,
syscalls, or cryptography (tree-sitter, bundled SQLite, tokio) is expected and is
reviewed by hand when a dependency is added rather than gated in CI.

## License

SlugAudit is free to use as a development tool, including on commercial software. From
[`LICENSE`](LICENSE):

> You may use SlugAudit for free to develop, audit, test, and maintain your own
> software—including commercial software—and you may sell the software you build
> with it. This covers software you create while using SlugAudit as a tool; it does
> not cover selling, embedding, bundling, or redistributing SlugAudit itself, or a
> modified version of it, as part of a commercial product or service. You do not need
> to contact SlugThugLabs for tool use.
>
> Contact admin@slugthuglabs.dev only if you want to embed, bundle, redistribute,
> sublicense, or otherwise make SlugAudit itself part of a commercial product or
> service. The permission summary above is explanatory; the terms below control.

The binding terms are the PolyForm Noncommercial License 1.0.0 plus the SlugAudit
Tool-Use Additional Permission, both carried in [`LICENSE`](LICENSE). The additional
permission states:

> Notwithstanding the Noncommercial Purposes limitation in the license below,
> SlugThugLabs grants you permission to run, reproduce, and modify SlugAudit
> internally as a development tool to analyze, audit, develop, test, or maintain
> any other software or project, including commercial products and services.

That permission does not cover distributing or embedding SlugAudit itself; those uses
need a separate written commercial license (`admin@slugthuglabs.dev`).
