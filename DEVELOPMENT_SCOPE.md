# Repository and Product Scope

## End-user product

The end-user product is the single `slugaudit` binary.

Users install and configure this binary with their AI agent. It runs as an MCP
stdio server and indexes the user's own projects.

## Runtime customer-project data

When running against an enabled customer project, SlugAudit creates and owns:

```text
<project-root>/.planning/slugaudit/
```

The primary derived index is:

```text
<project-root>/.planning/slugaudit/project.db
```

SlugAudit must exclude its own `.planning/slugaudit/` directory from discovery.
The database and its sidecar files are disposable derived data and can be
rebuilt from the project's source files.

The rest of the customer's `.planning/` directory is customer project data. It
is not development-only and may be indexed like any other project content.

## This repository's development files

In this repository, `.planning/` contains SlugAudit's internal engineering
specifications, architectural source of truth (`ARCHITECTURE.md`), design decisions,
and roadmap (the internal engineering wiki / RFCs).

The following are development-only and are not shipped in end-user binary releases:

- repository-level `.planning/` documentation
- `tests/`
- `benches/`
- `src/bin/check_*`
- `.github/` workflows
- audit, planning, coverage, and performance artifacts

Only `slugaudit` is the end-user executable. The `check_*` binaries are
quality and CI tools and are not required by an AI agent using the product.
The supported runtime targets are Linux and macOS; unsupported operating
systems fail closed before opening the SQLite database.

## Audit rule

When auditing this repository:

- Consult `.planning/ARCHITECTURE.md` as the authoritative source of truth for architectural design, threat model, and system invariants.
- Score `slugaudit` runtime behavior as product functionality.
- Treat `<customer-project>/.planning/slugaudit/` as runtime application state.
- Treat the rest of a customer's `.planning/` directory as customer data.
- Do not count development-only check binaries as end-user runtime code.
- Do not treat this repository's internal planning documents as shipped product binaries (they are developer documentation, not packaged runtime code).
- Distinguish CI/release-process failures from runtime production failures.
