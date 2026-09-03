# SlugAudit

### Stop making the AI read the same repository file after file.

## What this is

**SlugAudit is an MCP server for AI coding agents.** It indexes a codebase
once, keeps that index current, and exposes it to the agent as a set of
queryable tools. The agent uses those tools to answer questions about the
codebase — where things are defined, how they're called, what changed —
without re-reading the whole repository first.

## Who it's for

- **AI coding agents** (Claude Code, Codex, Grok, Bob, and any other MCP
  client) — they're the ones who use the tools. That's who the product is
  built for.
- **Humans** — you install it once and ask your agent to work. You never
  interact with SlugAudit directly beyond that setup; it's invisible during
  normal use.

## What it's not

- **Not a standalone audit tool.** There is no command that prints an audit
  report. The released binary is an MCP server; an AI agent starts it and
  calls its tools.
- **Not an auditor.** SlugAudit does not judge code, assign scores, or find
  bugs. It supplies facts; **the AI does the judging**. If you ask your
  agent "audit this repo," the agent reads SlugAudit's evidence and does the
  analysis — that division of labor is the whole point.

## How it works in one paragraph

You ask your agent to audit the codebase. The agent calls SlugAudit's tools
to learn what's in the repo — files, symbols, calls, imports, structure —
reads only the specific files and lines that matter, and spends its context
on the actual audit instead of on rediscovering where everything is.
SlugAudit gathers facts; the AI does the thinking.

## See it in action

You ask an agent: *"Where is login rate-limited, and is it applied
everywhere it should be?"*

```
[agent calls report]      → 214 files · 4 touch auth
[agent calls query]       → find every fn authenticate( and its call sites
[agent calls structure]   → match the AST shape that guards each request
```

The agent reads only the handful of files/line ranges that matter, instead
of re-reading the repository to rediscover where everything is. Its context
goes to reasoning, not housekeeping.

## Why SlugAudit

- **Saves AI context.** Discovery happens once and stays current, so agents
  stop re-listing files and re-tracing imports every session.
- **Keeps findings trustworthy.** Conclusions are tied to the source version
  they came from and invalidate automatically when those lines change.
- **Does not box you in.** It plays well with existing tools and exposes
  everything through read-only SQL, so you keep full control.
- **Free for your work.** Use it to build and sell your own software,
  including commercial projects.

## The point

During a large audit, an AI can burn most of its context on work like:

- listing and categorizing files over and over;
- finding every definition and use of a symbol;
- tracing imports between files;
- rereading unchanged source to recover context; and
- checking whether its previous understanding is stale.

SlugAudit does that discovery work once and keeps it current. After setup,
it should be invisible: the AI calls its MCP tools when useful, while the
user continues working normally.

The useful division of labor is simple:

- **SlugAudit gathers facts:** indexes, extracts, synchronizes, and searches.
- **The AI does the audit:** interprets evidence, finds real issues, judges
  severity, and recommends changes.

## Product boundary

The single end-user product is the `slugaudit-mcp` binary. Users install and
configure it with their AI agent, and the binary operates on the user's own
projects.

For each enabled user project, SlugAudit owns only this derived-data directory:

```text
<user-project>/.planning/slugaudit/project.db
```

The surrounding `.planning/` directory belongs to the user's project workflow.
Its other files are ordinary user project data and may be indexed normally.
SlugAudit excludes its own `.planning/slugaudit/` directory from discovery.

This repository also contains development-only material used to build and
validate SlugAudit: repository planning documents, tests, benchmarks, CI
workflows, and the `check_*` quality-gate binaries. Those files are not part
of the end-user product or shipped runtime binary.

## Quick start (downloaded binary)

The release asset is an **MCP server**, not a standalone command that prints
an audit report. Your AI agent starts it automatically over stdio and uses its
tools as a background repository index. You install and connect it once;
normal use should not require a separate SlugAudit workflow.

### 1. Download and verify it

From the GitHub release, download `slugaudit-mcp-x86_64-unknown-linux-gnu`
and `SHA256SUMS`. The published binary is currently for 64-bit Linux.

```bash
chmod +x slugaudit-mcp-x86_64-unknown-linux-gnu
sha256sum -c SHA256SUMS
```

The checksum command must report `OK`. If it does not, download the files
again and do not run the binary.

### 2. Install it at a stable path

Run the binary's setup menu:

```bash
./slugaudit-mcp-x86_64-unknown-linux-gnu menu
```

Choose **1) Install the binary**. This copies it to:

```text
~/.slugthug/bin/slugaudit-mcp
```

The stable path means your agent will continue to launch SlugAudit after you
move or delete the downloaded release file. You can check the installation:

```bash
~/.slugthug/bin/slugaudit-mcp version
```

### 3. Connect it to your AI agent

From the menu, choose **2) Connect to an AI agent**, or run the command
explicitly:

```bash
~/.slugthug/bin/slugaudit-mcp connect claude  # Claude Code
~/.slugthug/bin/slugaudit-mcp connect codex
~/.slugthug/bin/slugaudit-mcp connect bob
~/.slugthug/bin/slugaudit-mcp connect grok
```

To select interactively, omit the agent name:

```bash
~/.slugthug/bin/slugaudit-mcp connect
```

`connect` registers a global `slugaudit` stdio server using the agent's own
CLI. Verify it with the corresponding command, for example:

```bash
claude mcp list
# or: codex mcp list, bob mcp list, grok mcp list --scope user
```

Restart an already-running AI session so it reloads its MCP servers.

### 4. Use your AI agent normally

After the connection is registered, start a fresh AI session and work
normally. SlugAudit's project-control operation is an internal MCP operation:
the agent should enable the current project when it first needs the index.
You should not need to manage the database, run imports, or repeat setup.

Once indexed, the agent can use `report` for a compact repository snapshot
and `query`/`structure` to locate the exact files and lines it needs before
reading source. The result is less repetitive file reading and more context
available for actual reasoning.

### 5. Update to a new release

SlugAudit is a single binary, and upgrades are just replacing that binary at
the same path. To update an already-installed copy to the latest release:

```bash
~/.slugthug/bin/slugaudit-mcp update
```

`update` fetches the latest GitHub release (via `curl`), verifies its SHA-256
checksum, and atomically replaces the binary in place. It targets the same
stable path `connect` registered, so your agent configs keep working with no
re-connecting. Restart any running AI session to launch the new binary. If
no newer release exists, it reports that you're already up to date.

### Building from source

If you cloned this repository instead of downloading a release:

```bash
cargo build --release --locked --bin slugaudit-mcp
./target/release/slugaudit-mcp menu
```

For setup details and manual configuration for other MCP clients, see the
[connection guides](docs/).

## What the AI gets

| Need | SlugAudit supplies |
|------|--------------------|
| Understand the repository quickly | File/language counts and a compact report |
| Find code without scanning every file | Symbols, calls, imports, and structural matches |
| Trace relationships | Resolved, external, and unresolved dependency edges |
| Stay current while editing | Hashes, incremental synchronization, and freshness checks |
| Avoid repeating conclusions | Findings tied to the source version that produced them |

## Tools

| Tool | What it does |
|------|-------------|
| `report` | File/language counts, parser failures, evidence kinds, open findings |
| `query` | Read-only SQL against the project's indexed evidence |
| `structure` | Tree-sitter structural pattern matching (300+ languages) |
| `finding` | Persist an AI-reviewed conclusion (auto-invalidates on file change) |
| `finding_read` | Retrieve findings scoped to the current agent session |
| `project_control` | Internal project activation/control; normally used by the agent |
| `health` | Watcher health, sync status, tool-call counters |

## How it works

1. **Discovery** — walks the project tree, respects `.gitignore`/`.ignore`,
   skips VCS internals and SlugAudit's own data directory.
2. **Sampling** — reads each file, hashes it (BLAKE3), detects language,
   runs tree-sitter extraction (symbols, imports, Rust call sites, comments, diagnostics).
3. **Publishing** — writes everything into `.planning/slugaudit/project.db`
   in one atomic transaction with compare-and-swap concurrency control.
4. **Watching** — a filesystem watcher tracks changes. Incremental reconcile
   re-hashes only dirty files; full re-verification runs when the watcher
   is untrusted.
5. **Querying** — the AI calls `query` with SQL to find exactly which files
   and lines matter, then reads only those.

The database is disposable derived data — delete it and any tool call
rebuilds it from source.

## Repository scope

See [DEVELOPMENT_SCOPE.md](DEVELOPMENT_SCOPE.md) for the distinction between
this repository's development materials and the runtime behavior of SlugAudit
inside a user's project.

## Repository Structure & The `.planning/` Directory

All architectural blueprints, design decisions, and runtime index data live inside `.planning/`:

- **Source of Truth**: [`.planning/ARCHITECTURE.md`](.planning/ARCHITECTURE.md) is the canonical, authoritative source of truth for the codebase. It details the complete module map, data flow, process lifecycle, security boundaries, and the 10 core architectural invariants governing SlugAudit.
- **Design & Planning History**: [`.planning/archive/`](.planning/archive/) preserves historical planning records, architectural decision logs, and dependency inventories.
- **Disposable Runtime Cache**: `.planning/slugaudit/project.db` houses the SQLite index and evidence tables. This is derived data that SlugAudit automatically creates and synchronizes—it can be deleted at any time and any tool call will rebuild it from source.

Keeping these artifacts organized inside `.planning/` keeps the repository root uncluttered while providing both human developers and AI coding agents a dedicated, structured home for all architecture and design documentation.

## Documentation

- **[Architecture](.planning/ARCHITECTURE.md)** — the authoritative architectural specification (module map, data flow, security model, design FAQ)
- **[Connection guides](docs/)** — agent-specific setup for Bob, Claude Code, Grok, and Codex
- **[Archive](.planning/archive/)** — historical planning docs, decision log, dependency inventory

## Development

Requires Rust 1.97.1+ (edition 2024) on Linux or macOS. Run the same gates CI uses:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib --bins --tests --all-features
cargo run --quiet --bin check_source_limits --locked
cargo run --quiet --bin check_docs_drift --locked
cargo run --quiet --bin check_no_duplicates --locked
```

`#![forbid(unsafe_code)]` is enforced crate-wide. Zero unsafe in `src/`.

### Structured Logging

Diagnostics log to `stderr` with ANSI colors disabled by default. For automated log aggregators (e.g. Datadog, Grafana Loki, CloudWatch), set:

```bash
export SLUGAUDIT_LOG_FORMAT=json
```

All `stderr` events will be formatted as single-line JSON while `stdout` remains strictly JSON-RPC.

## License

SlugAudit is **free to use** — including for your own commercial software.
Use it to develop, audit, test, and maintain whatever you build, and you may
sell the software you create with it. Software you make using SlugAudit (and
the results it produces) does not become subject to SlugAudit's license just
because the tool was used.
The only thing you need to contact us about (`admin@slugthuglabs.dev`) is
distributing **SlugAudit itself** — for example embedding, bundling,
redistributing, sublicensing, or selling the tool as part of another product
or service. That includes a company wanting to make SlugAudit a branded part
of something they sell — we'd love to talk. Internal team and dev use is
always free.

See the complete [LICENSE](LICENSE) for the binding terms.
