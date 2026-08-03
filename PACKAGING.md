# Packaging and installation

SlugAudit is **Phase 0 foundation** software (see `README.md`). There is no
published crate, no tagged release, and no distributed binary as of this
writing. Everything in this document describes building and running the
project from a source checkout today, plus the gaps that exist because a
real release process doesn't exist yet. Nothing here is aspirational —
where a capability doesn't exist, that's stated plainly instead of being
described as if it worked.

## 1. Prerequisites and supported platforms

- **Rust toolchain**: exactly `1.97.1`, pinned in `rust-toolchain.toml`
  (`profile = "minimal"`, with `rustfmt` and `clippy` components). If you
  have `rustup` installed, running any `cargo`/`rustc` command inside the
  repository automatically fetches and uses `1.97.1` — no manual toolchain
  selection is required. Edition `2024` is required by `Cargo.toml`.
- **`#![forbid(unsafe_code)]`**: enforced at the crate root of
  `src/main.rs`/`src/lib.rs`. This is a source-level guarantee about
  SlugAudit's own code; it has no effect on how the binary is installed or
  run, and doesn't constrain end users in any way.
- **Operating system support, precisely**:
  - CI (`.github/workflows/quality.yml`) only builds and tests on
    `ubuntu-latest`. Linux is the only platform this project currently
    verifies in an automated way.
  - The codebase has no Linux-only APIs outside one narrow spot:
    `src/store/connection.rs` sets owner-only (`0600`) permissions on a
    newly created SQLite database file using a Unix-only syscall path
    (`#[cfg(unix)]`). On any non-Unix target (Windows), that function is a
    `#[cfg(not(unix))] ... Ok(())` no-op — the file is still created and
    the server still works, but its permissions are left at whatever the
    operating system default is, with no attempt to tighten them. In other
    words: **Windows is expected to build and run, but gets a strictly
    weaker file-permission guarantee on the project database than Linux or
    macOS, and this has not been verified by CI or manually in this
    project.** Symlink rejection for the database path
    (`SQLITE_OPEN_NOFOLLOW` plus a `symlink_metadata` pre-check) and for
    the `.planning/slugaudit` activation directory are plain `std::fs`
    calls with no `cfg(unix)` gate, so those protections apply on every
    platform Rust's standard library supports symlink detection on,
    including Windows.
  - macOs is not explicitly tested in CI either, but nothing in the
    codebase is Linux-specific beyond what's described above, so macOS is
    expected to behave the same as Linux (both are Unix for the purposes
    of `#[cfg(unix)]`).

## 2. Building and installing

There are no releases yet: `git tag -l` returns nothing in this
repository, and `Cargo.toml`'s `version = "0.1.0"` has never been
published to crates.io or anywhere else. **The only way to install
SlugAudit today is building it from source.**

```bash
git clone <this repository>
cd slugaudit-mcp-rust
cargo build --release
```

The resulting binary is at `target/release/slugaudit-mcp` — the name is set
by the `[[bin]]` section in `Cargo.toml`, which overrides the package name
(`slugaudit-mcp-rust`) that Cargo would otherwise derive the binary name
from.

There is no `cargo install` path published yet (that would require a
crates.io release, which doesn't exist), and no prebuilt binaries are
attached anywhere. A locked build (`cargo build --release --locked`, which
is what CI runs) is recommended to guarantee you get the exact dependency
versions recorded in the checked-in `Cargo.lock` rather than a possibly
different resolution.

## 3. MCP client registration

SlugAudit speaks the Model Context Protocol over **stdio only** —
confirmed directly in `src/main.rs`, which calls
`SlugAuditServer::new().serve(stdio())` from `rmcp::transport::stdio`.
There is no HTTP/SSE transport, no socket, and no other way to talk to it.

`src/main.rs` takes **no command-line arguments** and reads **no
environment variables** (confirmed by inspection — there is no
`std::env::args()` or `std::env::var()` call anywhere in `src/main.rs` or
`src/server.rs`). Every behavior the server has (which project it's
operating on, etc.) is driven entirely by the path arguments passed inside
individual MCP tool calls, not by process-level configuration. Concretely,
that also means the `tracing-subscriber` `env-filter` feature this project
depends on is not currently wired up to `RUST_LOG` or any other
environment variable in `main.rs` — logging verbosity is not
runtime-configurable today.

A typical MCP client config entry (the exact JSON shape varies by client —
this shows the common `"mcpServers"` object shape used by several
MCP-compatible clients, including Claude Code's `.mcp.json`):

```json
{
  "mcpServers": {
    "slugaudit": {
      "command": "/absolute/path/to/slugaudit-mcp/target/release/slugaudit-mcp"
    }
  }
}
```

No `args` or `env` entries are needed. Use an absolute path — the binary
isn't installed anywhere on `PATH` by this project's build process.

## 4. Enabling and disabling a project

SlugAudit's crate deliberately does **not** implement project
activation/deactivation itself — see README.md's "Activation ownership"
section for the full rationale. `src/project/activation.rs` only ever
*reads* an activation marker; it contains no code path that creates or
removes one. Owning that human-facing control is left to whatever host
application eventually embeds this MCP server.

Until that host application exists, here is the precise manual workaround
for enabling a project during development or manual testing today:

```bash
mkdir -p /path/to/your/project/.planning/slugaudit
```

That's it — `src/project/activation.rs` walks up from the path given in
each tool call looking for a directory literally named `.planning/slugaudit`
(the constants are `PLANNING_DIR = ".planning"` and
`ACTIVATION_DIR = "slugaudit"`), and treats its existence as the sole
"this project is enabled" signal. The nearest ancestor directory
containing that marker becomes the project root.

**To disable a project**, remove that same directory:

```bash
rm -rf /path/to/your/project/.planning/slugaudit
```

This is a manual workaround for development and testing use today, **not**
the intended long-term human-facing UX described in `ARCHITECTURE.md` and
`CLAUDE.md` — the real interface is a single human-facing enable/disable
control owned by a future host application, not a directory a person is
expected to create by hand.

## 5. Database location and permissions

Every active project gets exactly one SQLite database file, and its
location is not configurable — `src/project/database_path.rs` computes it
as:

```
<project root>/.planning/slugaudit/project.db
```

(`database_path()` joins the activation directory computed by
`src/project/activation.rs` with the fixed filename `project.db`; there is
no argument or environment variable that can move it elsewhere.)

Permissions, from `src/store/connection.rs`:

- **On Unix** (Linux, macOS): a newly created database file is opened with
  `O_CREAT | O_EXCL` and mode `0600` (owner read/write only) in a single
  syscall, so there is no window where the file briefly exists with wider,
  umask-derived permissions before being tightened. An **existing** file's
  permissions are never changed — SlugAudit never widens or narrows
  permissions on a database file it didn't just create itself, to avoid a
  TOCTOU race against whatever process actually created it. Verified
  directly in this pass: creating a fresh project and making one real tool
  call against it produced a database file with mode `600`.
- **On non-Unix** (Windows): permission tightening is a no-op (see
  section 1) — the file is created with whatever default permissions the
  OS/filesystem applies, and SlugAudit makes no attempt to restrict access
  further.
- On every platform, the database path is checked against being a symlink
  before opening (`symlink_metadata` pre-check plus
  `SQLITE_OPEN_NOFOLLOW`), so a symlinked `project.db` path is refused
  rather than followed.

## 6. Upgrades, schema compatibility, rollback, and removal

Schema versioning lives in `src/store/migrations.rs`: the current schema
is version `1` (`CURRENT_SCHEMA_VERSION`), tracked via SQLite's own
`PRAGMA user_version`. Migrations are **forward-only**:

- Opening a database at the current version is a no-op.
- Opening a database at an older version applies the current schema DDL
  and records the new version, atomically, in one transaction (so a crash
  mid-migration can't leave tables created but the version still reporting
  the old number).
- Opening a database at a **newer** version than the running binary
  supports is a hard, enforced error
  (`MigrationError::UnsupportedVersion`) — the binary refuses to open it
  at all, rather than guessing at compatibility.

**There is no downgrade or rollback path.** If a database has been opened
by a newer SlugAudit build that migrated it to a schema version an older
build doesn't know about, that older build cannot open it again — this is
enforced by the check above, not just undocumented behavior. There is
currently only one schema version, so this hasn't been exercised in
practice yet, but the mechanism is real and will matter starting with the
first schema change.

**Removal**: delete the activation directory entirely —

```bash
rm -rf /path/to/your/project/.planning/slugaudit
```

This simultaneously disables the project (per section 4 — activation and
the database live under the same directory) and permanently discards the
database, including every stored finding. **There is no separate
backup/export tool.** If you want to preserve findings before removing a
project, you need to either:

- copy the `project.db` file directly before deleting the directory
  (`cp /path/to/project/.planning/slugaudit/project.db ~/backup.db`), or
- read data out via direct SQLite access (`sqlite3 project.db "SELECT ..."`)
  or via the `query` MCP tool, before removal.

Neither of these is a built-in "export" feature — they're just manual use
of the fact that the data lives in an ordinary SQLite file you have direct
access to.

## 7. Clean-machine install verification

**What was actually done in this pass** (this sandbox's toolchain was
already present and matched the `rust-toolchain.toml` pin exactly — this
was not a from-scratch OS install — but this worktree's `target/`
directory was genuinely absent beforehand, so the build below is a real,
non-cached, from-scratch build, not a re-run against warm build state):

```
$ cargo build --release --locked
   [... full dependency compilation, including tree-sitter-language-pack's
        build-time download of parser sources over the network ...]
    Finished `release` profile [optimized] target(s) in 52.97s
```

producing a working `target/release/slugaudit-mcp` binary (12,057,072
bytes). That binary was then driven with a real raw JSON-RPC-over-stdio
script (no test harness, no mocked transport) against a scratch project
activated the way section 4 describes:

```
$ mkdir -p /scratch/smoke_project/.planning/slugaudit
$ echo 'pub fn a() {}' > /scratch/smoke_project/lib.rs
$ python3 smoke_test.py   # spawns the binary, writes/reads raw JSON-RPC lines
RAW INITIALIZE RESPONSE:
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18", ...
 "serverInfo":{"name":"rmcp","version":"2.2.0"}, "instructions": "..." }}

RAW REPORT TOOL CALL RESPONSE:
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text",
 "text":"{\"evidence_counts\":[{\"count\":1,\"kind\":\"Structure\"},
 {\"count\":1,\"kind\":\"Symbol\"}],\"file_count\":1,
 \"languages\":[{\"file_count\":1,\"language\":\"rust\"}],
 \"open_finding_count\":0,\"parser_failure_count\":0,
 \"revision_id\":\"rev-1\"}"}], "isError":false}}

Exit code: 0
```

The resulting database file was confirmed to have mode `600`
(`stat -c "%a" .planning/slugaudit/project.db` → `600`), matching section 5.

**What was not verified in this pass**: a literal clean-OS install
(fresh container/VM with no Rust toolchain at all, installing `rustup`
from scratch, letting it pick up `1.97.1` from `rust-toolchain.toml` for
the first time) was not attempted, because this sandbox already had a
matching toolchain installed. That step is lower-risk (rustup's toolchain
pinning is well-established, standard behavior) but is still genuinely
untested by this pass. If you want to verify it yourself, the procedure
is:

1. Start from a machine/container with no Rust toolchain installed.
2. Install `rustup` (https://rustup.rs).
3. `git clone` this repository and `cd` into it — `rustup` will read
   `rust-toolchain.toml` and fetch `1.97.1` automatically on the first
   `cargo`/`rustc` invocation.
4. `cargo build --release --locked`.
5. `mkdir -p /path/to/a/scratch/project/.planning/slugaudit` and put a
   trivial source file in it.
6. Register the built binary with a real MCP client (section 3), or run a
   manual smoke test modeled on `tests/stdio_protocol.rs` — spawn the
   binary, write an `initialize` JSON-RPC request line to its stdin, and
   confirm a well-formed response line comes back on stdout.

## 8. Checksums and provenance for release artifacts

There are no release artifacts to check: no tags exist, `Cargo.toml`'s
version has never been published, and CI (`.github/workflows/quality.yml`)
has no release job that produces or uploads binaries. This is a known gap,
not an oversight to route around: **when a first real release is cut**, it
should ship SHA-256 checksums for each build artifact, and ideally either
a reproducible build process or SLSA provenance attestation. None of that
exists yet because there is nothing to attest to — building that tooling
now, ahead of an actual release process, would be solving a problem this
project doesn't have yet.
