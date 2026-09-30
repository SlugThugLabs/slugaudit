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

## Connect an agent

`slugaudit connect` writes a `slugaudit` MCP server entry into an agent's or editor's
own configuration. With no argument it lists the agents detected on the machine to
pick from; with a name it configures that target directly.

```bash
slugaudit connect claude     # Claude Code (~/.claude.json)
slugaudit disconnect claude  # alias: slugaudit remove claude
```

26 targets in this revision (ids are the `connect` argument; the display name is in
parentheses where the id is not self-evident):

`agy` (Antigravity), `amp`, `bob`, `claude` (Claude Code), `claude-desktop`, `codex`,
`copilot` (GitHub Copilot), `crush`, `cursor`, `droid` (Droid / Factory), `gemini`
(Gemini CLI), `goose`, `grok`, `hermes`, `kimi` (Kimi Code), `omp` (Oh My Pi),
`opencode`, `openhands`, `pearai`, `pi`, `q` (Amazon Q Developer), `trae`, `warp`,
`windsurf`, `zed`, `1mcp`.

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
| `report` | Project snapshot: file and language counts, parser failures, evidence-kind counts, open findings. |
| `leads` | Unranked mechanical facts: call sites by callee (`unwrap`, `expect`, `panic`, `todo`, `unimplemented`, `assert`), diagnostics, `TODO`/`FIXME`/`HACK` markers, credential-shaped files. Counts by default; paged file/line rows for one `mechanic` on request. Callee-based mechanics cover Rust only in this revision. |
| `query` | Read-only SQL over the index (`files`, `evidence`, `dependency_edges`), including full file text via `files.content`. |
| `structure` | Tree-sitter AST queries in one file or across a language, returning matched nodes with line spans and snippets. |
| `finding` | Persists a conclusion the agent has reviewed, bound to the file's current content hash — it goes stale when that file changes. The judgment is the agent's; SlugAudit never generates it. |
| `finding_read` | Findings written by the current session only, never another session's conclusions. |
| `project_control` | Enable (`on`) or disable (`off`) a project, optionally setting its profile; `off` purges the database. |
| `health` | Operational snapshot: watcher health, unreconciled event counts, current revision and file count, parser-pack version, tool-call counters. |

Tool schemas, payload keys, and usage guidance are served to the agent at runtime by
the server itself (`INSTRUCTIONS` in `src/server.rs`).

## Resources

Six pull-based MCP resources, read with `resources/read`. They cost an agent nothing
in context unless it asks for one. The five query resources are also in the repository
under [`examples/queries/`](examples/queries/), and are compiled into the binary from
those files:

- `slugaudit://queries/README.md` — SQL and Tree-sitter recipes ([`examples/queries/README.md`](examples/queries/README.md))
- `slugaudit://queries/find_unhandled_panics.sql` — `unwrap` / `expect` / `panic` call sites
- `slugaudit://queries/auth_attack_surface.sql` — functions, methods, and calls touching auth, tokens, or sessions
- `slugaudit://queries/syntax_diagnostics.sql` — syntax-error and parser diagnostics per file
- `slugaudit://queries/rust_functions.scm` — Tree-sitter pattern for Rust functions
- `slugaudit://schema.sql` — the index schema (from `src/store/schema.sql`)

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
and its variable are documented in `src/model/limits.rs`. An unset or unparseable
override is ignored, leaving the default in place.

Logging goes to `stderr`; `stdout` carries JSON-RPC only. Set
`SLUGAUDIT_LOG_FORMAT=json` for one-line JSON log events.

## Documentation

- [AGENTS.md](AGENTS.md) — why this exists, the founding metric, non-negotiables, verified conventions
- [.planning/ARCHITECTURE.md](.planning/ARCHITECTURE.md) — design, invariants, data flow, module map
- [.planning/PLAN.md](.planning/PLAN.md) — the active plan
- [.planning/archive/DECISIONS.md](.planning/archive/DECISIONS.md) — decision log (append-only history)
- [docs/CLIENTS.md](docs/CLIENTS.md) — every `connect` target, its config path, and its scope
- [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) — operator runbook

## Development

Rust 1.97.1 (edition 2024). The same gates CI runs:

```bash
cargo fmt --all -- --check
cargo run --quiet --bin check_source_limits --locked
cargo run --quiet --bin check_no_duplicates --locked
cargo run --quiet --bin check_docs_drift --locked
cargo run --quiet --bin check_producer_coverage --locked
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --bins --tests --all-features --locked
cargo run --quiet --bin check_coverage --locked
```

CI also runs `cargo-deny` (advisories, bans, sources, licences), a RustSec audit, and
scoped mutation testing. `check_performance` compares the criterion benches against the
committed baseline at `.planning/archive/perf_baseline.json`; it is not part of the CI
job. `check_coverage` gates line coverage at 83% by default — the threshold is
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
