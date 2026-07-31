# SlugAudit MCP

SlugAudit is a database-backed evidence index built for AI code auditors. It
uses Tree-sitter to pre-parse supported source files and lets an AI search,
retrieve, and connect evidence without repeatedly opening hundreds of flat
files. SlugAudit does not decide whether code is correct; the AI still performs
the audit judgment.

**Zero setup by default.** With no PostgreSQL configured, SlugAudit stores
its index in a per-project SQLite file — nothing to install, nothing to run.
Configure PostgreSQL (see "Backends" below) if you want a shared index one
server serves across multiple developers or machines; if you never do,
you'll never notice it's an option.

**This is infrastructure for an LLM, not a tool a human runs commands
against.** Every tool in the "MCP tools" table below is called by the AI
model — the same way it would call a built-in `Read` or `Grep` tool — never
typed by a person. The *only* human-facing surface, at all, is the two-word
activation toggle in the next section. If you're adding something a human is
meant to invoke directly, it almost certainly belongs somewhere other than a
new MCP tool here.

## Licensing

SlugAudit is licensed under the [PolyForm Noncommercial License
1.0.0](LICENSE) — free to use, modify, and redistribute for any
noncommercial purpose (personal projects, research, internal tooling at a
nonprofit or educational institution, etc.). No commercial use is licensed
under these terms: incorporating SlugAudit into a commercial product or
service, or otherwise using it for commercial advantage, requires a separate
commercial license.

If you want to use SlugAudit commercially, reach out first: **licensing@slugthuglabs.dev**.

## Project contract

A project is enabled by the presence of `.planning/slugaudit/`. An integrating
client may expose the only project-level human command:

```text
/slugaudit on
/slugaudit off
```

`on` creates the directory. `off` purges that project's database evidence and
then removes the directory. The reusable adapter functions are
`app.activation.enable_project` and `app.activation.disable_project`.

There are no human commands for importing, syncing, rebuilding, parsing,
changed files, or database maintenance. Before every AI tool query SlugAudit:

1. Discovers and hashes the complete supported, non-ignored source set.
2. Proves the local manifest and published database revision agree.
3. Parses and imports added files.
4. Replaces every derived fact for modified files.
5. Purges deleted files and obsolete derived evidence.
6. Rebuilds dependency edges and publishes one atomic current revision.
7. Answers only from that verified revision.

Any discovery, parsing, database, or state failure fails the query. There is no
stale fallback.

## MCP tools

| Tool | AI use |
|---|---|
| `audit_overview` | Project statistics, languages, and compact tree |
| `audit_search` | Case-insensitive literal or constrained regex search |
| `audit_read_file` | Retrieve indexed source by project-relative path |
| `audit_dependents` | Trace incoming blast radius or outgoing dependencies |
| `audit_file_tree` | Browse the complete indexed source tree |
| `audit_brief` | Compact project-wide risk leads and open findings |
| `audit_finding` | Persist an AI-reviewed conclusion against current evidence |
| `audit_raw_sql` | Constrained, project-scoped, database-enforced read-only query (PostgreSQL only — see "Backends") |

Every successful response includes `slugaudit_meta` with the contract version,
schema version, project ID, revision ID, manifest hash, sync timestamp, and
`freshness: verified`. Automated risk patterns (below) are leads, not findings
or scores — the AI still has to review and decide, then persist a real
conclusion with `audit_finding` if one holds up.

## Language support — what actually works, honestly

Eight languages, each via its own Tree-sitter grammar (`languages/<name>.py`).
Every one of them extracts **signatures** (functions, classes, and similar
top-level constructs) and **imports** (raw import statements plus best-effort
resolution to a dependency graph edge between files), and every one has a
handful of **risk patterns** (regex-based leads like `eval`, `unwrap`,
`unsafe.Pointer` — see each extractor's `extract_risk_patterns` for the exact
list; these are leads, never findings).

| Language | Signature extraction | Import resolution | Known limitations |
|---|---|---|---|
| Rust | fn, struct, enum, trait, impl, type alias, const/static, macro | Cross-crate via a discovered workspace `crate_map`, `crate::`/`super::`/`self::` paths, re-export chasing (depth-capped) | Most rigorously built of the 8; no known gaps |
| Python | Functions (incl. decorated, async), classes | Relative (`from . import x`) and absolute (`from pkg.mod import x`) imports, resolved against real files on disk | None currently known — see `tests/test_import_resolution.py` for exact coverage |
| TypeScript / JS | Functions, classes, interfaces, enums, type aliases, `const`/`let`-bound functions (including async arrows) | Relative imports (`./foo`, `../bar`) | `tsconfig.json` path-alias imports (`@app/utils`) are **not** resolved and are classified external, same as a real npm package |
| Go | Functions, methods, structs, interfaces | Same-module imports via `go.mod`'s declared module path; resolves to a representative file in the target package directory | Struct/type signature text omits the leading `type` keyword (cosmetic — name/kind/resolution are unaffected). No `go.mod` means nothing resolves as internal at all (there's no reliable way to tell a same-module import from a third-party one without it) |
| Java | Classes, methods (incl. generics, annotations) | Package-path-to-file resolution against project root, `src/`, and `src/main/java/`; known JDK prefixes short-circuit to external without a filesystem check | None currently known |
| C | Functions | `#include "local.h"` (quoted) resolves against source-relative, project-root, and common include dirs; `#include <system.h>` (angle-bracket) always stays external, by C's own syntax | `#define` macros are **not** extracted as signatures at all |
| C++ | Classes (incl. templates), functions | Same quote/angle-bracket handling as C | Same macro gap as C. Templates are captured as plain classes/functions — no separate generic-parameter extraction |
| Ruby | Methods, singleton methods, classes, modules | `require_relative` (relative), plain `require` (project root / `lib/` / `app/`), `load`/`autoload` (same lookup as `require`) | `include`/`extend`/`prepend` reference an already-loaded constant, not a file — never resolve, which is correct, not a gap |

None of the "known limitations" above produce *wrong* data — they're
documented gaps in coverage (something real isn't captured), not
correctness bugs (something is captured but wrong). If you find one that
behaves incorrectly rather than incompletely, that's a bug — the resolution
test suite (`tests/test_import_resolution.py`, `tests/test_circular_imports.py`,
`tests/test_extractors.py`) is where a regression test for it belongs.

## Host-adapter protocol and the `_project_root` override

A native host integration (e.g. an editor's built-in agent, the same layer
that already mediates `Read`/`Write`/`Bash` for that agent) can bind every
tool call to whichever project is currently active by injecting a reserved
`_project_root` argument, and can drive `/slugaudit on|off` through a reserved
`_slugaudit_project_control` tool name. Neither name is advertised in the tool
schemas presented to the model, and every schema sets
`additionalProperties: false`.

In the expected deployment shape — a host that, like it does for its own
built-in tools, constructs the final tool-call arguments itself rather than
forwarding whatever raw JSON the model emits — the model has no path to ever
see or set either reserved name, and the section below is inert by
construction. SlugAudit cannot verify that from inside this codebase, though:
nothing about the MCP transport or JSON Schema *itself* guarantees a given
host enforces that. The mechanism below exists as a way to make that
guarantee explicit and checkable, for any host that doesn't, rather than as
evidence that this class of host actually needs it. Do not read the existence
of this section as SlugAudit treating its own calling model as an adversary
by default — treat it as an available lever, off by default, for a host that
turns out to need one. The lever:

- If `SLUGAUDIT_HOST_TOKEN` is **not set** (the default), `_project_root` is
  honored as sent, with a one-time startup log noting that this argument is
  currently unauthenticated.
- If `SLUGAUDIT_HOST_TOKEN` **is set**, `_project_root` is only honored when
  the same call also supplies a matching `_host_token` argument (compared in
  constant time). A missing or wrong token silently falls back to the
  server process's own working directory — the same as if `_project_root`
  had not been sent at all.

To close this gap, set `SLUGAUDIT_HOST_TOKEN` to a long random value in the
server's environment and configure your host adapter to send the same value
as `_host_token` alongside `_project_root`. If your deployment only ever
audits the single directory the server is launched in, you don't need
`_project_root` at all — leave it unset and the server always uses its own
`cwd`.

## Backends

SlugAudit resolves which database to use, in this order, every time a tool
call needs one:

1. Environment variables: `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`,
   `PGPASSWORD`.
2. `config.toml` (via `$SLUGAUDIT_CONFIG` or the default install path).
3. **Neither resolves → SQLite**, one file per project at
   `.planning/slugaudit/audit.db`. No installation, no server, no
   configuration — this is what a fresh clone gets with zero setup.

The two backends are not identical: the SQLite one is scoped to the 7 tables
and 7 tools (everything except `audit_raw_sql`) the current design actually
uses — see `repositories/sqlite/`'s module docstring for the deliberate
maintenance-burden tradeoff behind that choice. Every other tool behaves
identically either way.

**Switching from SQLite to PostgreSQL later is automatic.** The moment a
project that's been running on SQLite gets a working PostgreSQL
configuration, the next tool call syncs fresh into PostgreSQL and migrates
that project's findings over (the only thing SQLite held that isn't just
re-derived from source on every sync anyway), then renames the old file to
`audit.db.migrated` rather than deleting it. Nothing needs to be triggered
manually.

## Installation

SlugAudit requires **Python 3.11+** and runs on **Linux or macOS only** —
cross-process synchronization uses POSIX `fcntl` file locks, which do not
exist on Windows. `mcp_server.py` checks this at startup and fails with a
clear message rather than a raw `ImportError`.

```bash
./setup.sh
```

This creates a `.venv` and installs SlugAudit into it — `pyproject.toml` is
the single source of truth for dependencies (there is no separate
`requirements.txt` to keep in sync; a stale duplicate of the dependency list
is exactly how this project once shipped an outdated, CVE-affected `mcp` SDK
version in two different places at once). Nothing beyond this step is
required — see "Backends" above for the zero-config default.

Two optional convenience scripts register the installed server with a
specific AI client:

```bash
./claude-code-install.sh   # registers with Claude Code
./grok-install.sh          # registers with Grok (--scope user|project)
```

Both detect whether `config.toml` exists and register accordingly — with it,
PostgreSQL; without it, the zero-config SQLite default. Neither requires
`config.toml` to exist.

### PostgreSQL setup (optional — for a shared, multi-machine index)

PostgreSQL 15+ only if you choose to configure it:

```bash
cp config.toml.example config.toml
chmod 600 config.toml
$EDITOR config.toml
```

`config.toml` holds a database password in plain text. The server refuses to
load it if it is readable by anyone other than its owner.

## Verification

```bash
python3 -m pytest -q
mypy --strict app languages repositories services domain infrastructure mcp_server.py
ruff check app languages repositories services domain infrastructure mcp_server.py
git diff --check
```

A `.pre-commit-config.yaml` runs the same `ruff`/`mypy` checks (plus generic
hygiene: trailing whitespace, merge-conflict markers, large files) on staged
changes if you have [pre-commit](https://pre-commit.com) installed
(`pre-commit install` once per checkout).

`pytest -q` covers both backends. SQLite needs no gating — it's a Python
stdlib module, so `tests/test_sqlite_backend.py`, `tests/test_sqlite_end_to_end.py`,
`tests/test_sqlite_migration.py`, `tests/test_import_resolution.py`, and
`tests/test_circular_imports.py` all run real (non-mocked) database and
Tree-sitter behavior on every normal test run. PostgreSQL is the one backend
that needs a real network server, so everything touching it stays mocked
except `tests/test_integration_db.py`, which exercises the real schema
migration, advisory lock, connection pool, and a repository round trip
against a live PostgreSQL database. It is skipped by default and only runs
when `SLUGAUDIT_RUN_DB_TESTS=1` is set:

```bash
SLUGAUDIT_RUN_DB_TESTS=1 PGHOST=... PGDATABASE=... PGUSER=... PGPASSWORD=... \
  python3 -m pytest -q tests/test_integration_db.py
```

It creates and drops its own schema (`SLUGAUDIT_TEST_SCHEMA`, default
`slugaudit_test`) via `search_path` and never touches `public` — but still
point it at a database you're allowed to create/drop a schema in, never one
holding real audit evidence.
