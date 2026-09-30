# Security Policy

## Threat model

SlugAudit's design target is a **lazy or unsupervised AI agent, not a hostile
attacker**. The agent already has filesystem access, as the user that launched
it, to the same directory the database lives in — so SlugAudit grants it no
access it did not already have. The guards below defend against the accidental
version of that agent: a call that writes where it should not, a
credential-shaped file surfaced without being named, a database left readable
by other users on a shared machine. They are convenience guards, not a defence
against a malicious actor.

- **The stdio transport is the auth boundary.** The server speaks MCP over
  stdio only (`rmcp::transport::stdio`); stdout is the JSON-RPC transport and
  logs go to stderr. There is no network listener, no HTTP/SSE transport, no
  socket, and no multi-client multiplexing, so there is no remote surface to
  authenticate. The process is spawned as a child of the agent; only that
  parent can send it requests. A future network transport (SSE, WebSocket)
  would need its own authentication layer.
- **The activation marker is scope, not authorization.** A tool call resolves
  its project by walking up from the given path looking for
  `.planning/slugaudit/`; if it is absent the call fails with `NotActive`. Its
  purpose is "which project am I operating on?", not "may I?".
- **Correctness comes from the connection, not from query text.** The `query`
  tool opens the database `SQLITE_OPEN_READ_ONLY` and installs a SQLite
  authorizer that denies `ATTACH`/`DETACH`. There is no SQL keyword blocklist
  and no table allowlist (`src/store/connection.rs`, `src/tools/query.rs`).
- **`structure` executes agent-supplied Tree-sitter queries.** This is the most
  interesting surface, because the agent can ask SlugAudit to do work the human
  did not request.
- **`finding` stores agent-authored text verbatim, by design.** The human reads
  it directly. Findings are scoped to the writing session, and prior sessions'
  rows are purged on the first sync
  (`src/sync/manager_meta.rs::purge_prior_session_findings`), so a past session
  cannot leave commentary that misleads the next one.

Fuller discussion: `.planning/ARCHITECTURE.md`, *Security and trust model*.

## What the tool can read

The project's own source files, plus anything else within the project root that
discovery does not exclude — the same files the calling agent can already read.
Per-file byte limits and resource-profile caps apply (`src/model/limits.rs`).

SlugAudit is built to audit codebases, so it does not pretend secrets are
absent. `report` surfaces a `credential_pattern_file_count` and `leads` can
return the matching file paths, so the agent knows it is handling
credential-shaped content instead of encountering it silently
(`CREDENTIAL_PATH_PREDICATE`, `src/tools/mod.rs`). Query parameters, finding
text, and source contents are kept out of log emissions
(`src/tools/redaction_tests.rs`); the tool-call span logs only the tool name,
timing, and outcome. Logs go to stderr.

## The index: location, permissions, contents

SlugAudit owns exactly one derived-data directory per enabled project:

```text
<project>/.planning/slugaudit/project.db      # the index
<project>/.planning/slugaudit/project.db-wal  # SQLite sidecar
<project>/.planning/slugaudit/project.db-shm  # SQLite sidecar
```

The surrounding `.planning/` directory belongs to the project and is indexed
normally; SlugAudit's own `.planning/slugaudit/` is excluded from discovery and
is gitignored via `/.planning/slugaudit/`.

Unix permissions, enforced at creation:

| Path | Mode | Enforcement |
| :--- | :--- | :--- |
| `.planning/slugaudit/` (directory) | `0700` | Created owner-only; a pre-existing wider directory is tightened (`src/project/activation.rs`). |
| `project.db` | `0600` | Created owner-only in one syscall (`create_new(true).mode(0o600)`); an existing database readable or writable by group or other is **rejected**, never silently accepted (`src/store/connection.rs`). |
| `project.db-wal`, `project.db-shm` | `0600` | Created owner-only like the database (`src/store/connection_tests.rs`). |

**The index holds full source text.** `files.content` stores the text of every
indexed file, and the `query` tool can read it. Treat the directory as you
would the source it indexes: do not commit it, do not copy it to a shared or
backed-up machine, and remove it before handing a checkout to someone who
should not have the source. `project_control {"action":"off"}` purges it.

**The index is disposable.** No fact lives only in the index: the database is
rebuilt from project source on the next tool call. A database with a foreign or
unreadable schema version is discarded and rebuilt rather than migrated
(`src/store/migrations.rs`, `CURRENT_SCHEMA_VERSION`). Deleting it costs a
rebuild, not data.

Two further open-time guards: symlinks are rejected (`SQLITE_OPEN_NOFOLLOW`),
and databases on network filesystems — NFS, CIFS, SMB — are refused, because
WAL locking is unreliable across those protocols. Unknown platforms fail closed
(`src/store/connection.rs`, `src/store/netfs.rs`).

## Supported versions

Security fixes are released against the latest released version only. Release
artifacts are **Linux x86_64 only** — the platform CI verifies, and therefore
the only one shipped (`.github/workflows/release.yml`). The runtime supports
Linux and macOS (`AGENTS.md`); macOS has no prebuilt artifact and other
platforms fail closed at database open.

## Reporting a vulnerability

Use GitHub's private security advisories for this repository:

<https://github.com/SlugThugLabs/slugaudit/security/advisories/new>

Do not open a public issue for an unfixed vulnerability. No dedicated security
email address and no PGP key are published for this project, so the advisory
channel is the only supported reporting path. Please include the version
(`slugaudit --version`, or `Cargo.toml`), the platform, and a reproduction.
