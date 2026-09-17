# Production-Readiness Plan

Branch: `googlefi-code`. Status: proposed — no phase started.

## Purpose

Close the gap between SlugAudit's current engineering and a Google-grade production
bar. The review found the **runtime** engineering already strong (invariants, budgets,
failure semantics, deterministic output) and the **organizational and supply-chain
shell** largely absent (ownership, contribution/disclosure policy, release contract,
rollback, artifact provenance, fuzzing of untrusted inputs).

This plan is hygiene and correctness work. It adds no product features.

## Ground rules (repo coupling — violating these fails CI)

- Every phase ends with the full gate suite green (see Baseline).
- Adding, renaming, or deleting anything under `src/` requires updating the fenced
  module-map block in `.planning/ARCHITECTURE.md`.
- Adding or removing a `src/**/*_tests.rs` file requires updating the
  “The 59 `*_tests.rs` files share this pattern” sentence in `ARCHITECTURE.md`.
- Test function names and commit subjects must be **globally unique**
  (`check_no_duplicates`).
- New gates are pure-Rust cargo bins under `src/bin/` (repo idiom: no shell, no
  Python). Each new bin must be registered in `Cargo.toml`, added to
  `.github/workflows/quality.yml`, and listed in the ARCHITECTURE.md module map.
- `check_source_limits` ceilings apply: 200 code lines per production file; 200–300
  only with an annotation; test files cap at 500.

## Baseline (Phase 0)

Record current gate output before touching anything, so later regressions are
attributable rather than argued about.

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --bins --tests --all-features --locked
cargo run --quiet --bin check_source_limits --locked
cargo run --quiet --bin check_no_duplicates --locked
cargo run --quiet --bin check_docs_drift --locked
cargo run --quiet --bin check_coverage --locked          # record the number
cargo run --bin check_performance --locked               # record medians
```

**DoD:** baseline numbers recorded in this file (append below) or in the first
commit message body.

---

## Phase 1 — Store safety and secrets

**Goal:** the on-disk database is safe to create in someone else's repository.

Blocks pointing SlugAudit at a real employer's checkout.

### Work

1. **Verify, then enforce, on-disk permissions.** `src/store/connection.rs` already
   carries permission guards per its module docs — first establish what it does
   today, then make it explicit and tested: `.planning/slugaudit/` created `0700`,
   `project.db` and its sidecars `0600`.
   Files: `src/store/connection.rs`, `src/project/database_path.rs`;
   test in `src/store/connection_tests.rs` asserting mode bits on a fresh DB.
2. **Auto-ignore on enable.** Nothing in `src/` writes a `.gitignore` entry today
   (verified: the code only ever *reads* ignore files), so a customer's first
   `git status` shows an untracked binary blob plus sidecars. On
   `project_control {"action":"on"}`, append `.planning/slugaudit/` to the project's
   `.gitignore` when absent, and report that in the response.
   Files: `src/project/activation.rs`, `src/tools/project_control.rs`.
3. **Credential-content decision (needs a decision, not just code).** Files matching
   `CREDENTIAL_PATH_PREDICATE` are identified for `report`/`leads`, but their **full
   text is still stored** in `files.content`. Decide whether to withhold content for
   credential-shaped paths — recommendation: store hash/metadata, omit content, and
   mark the file so the agent knows the text is deliberately absent and must be read
   from disk. Record the decision as `DECISIONS.md`-style entry.
   Files: `src/tools/mod.rs` (predicate), `src/sync/sample.rs`,
   `src/sync/sample_batch.rs`.
4. **Document the exposure** in `SECURITY.md` (Phase 3): DB location, permissions,
   that it holds full source text, and that it is disposable.

### New enforcement

None (tests only). Permission bits and the gitignore entry are test-pinned.

### Acceptance

- `cargo test --lib -- store::connection` passes with explicit mode assertions.
- A temp project after `project_control on` has `.gitignore` containing the entry.
- Test proving a credential-shaped path stores no content.
- **Size:** S/M. **Depends on:** nothing.

---

## Phase 2 — Evidence truth

**Goal:** the index stops lying about what it does not have.

These are defects surfaced during review, not features. Each one currently produces a
*false verdict* rather than a gap: a skipped file is absent from `files`, therefore
absent from `known_paths`, therefore every import targeting it resolves to
`Unresolved` — which the briefing tells the agent to inspect as a finding.

### Work

1. **`known_paths` from discovered paths, not indexed rows.**
   `src/sync/revision_edges.rs::known_paths` runs `SELECT path FROM files`, while its
   own doc comment in `src/graph/mod.rs` promises “every path that will exist in the
   project”. Thread the discovered/skipped path set (already produced by
   `sample_batch` and carried in the reconcile report) into publish and reconcile.
   Test: an import targeting an oversize-skipped file resolves, not `Unresolved`.
2. **Existence rows for every discovered file.** A row in `files` should mean “this
   file is in the project”. Add a nullable `skip_reason`; give skipped files a
   **streaming** BLAKE3 identity (one pass, O(1) memory) so identity is cheap even
   when content is not retained. Audit every consumer that assumes
   `files` == indexed set — file counts, `report`, manifest hash — and make the
   distinction explicit.
   Files: `src/sync/sample.rs`, `src/sync/sample_batch.rs`, `src/sync/revision.rs`,
   `src/sync/sweep.rs`, `src/store/schema.sql` (+ `CURRENT_SCHEMA_VERSION` bump →
   discard-and-rebuild, which is free here).
3. **Emit `ExtractionCompleteness::ContentOnly`.** It exists in the enum and the
   schema `CHECK` and is **never constructed** (verified: it appears only in
   `src/model/parser.rs`, its `as_sql_text` arm, and `schema.sql`). Today “text
   present, not parsed” reports as `Unavailable` (`src/sync/analyze.rs`), which reads
   as “no content” while the content sits in `files.content`.
4. **Remove or produce the dead evidence kinds.** `Chunk`, `RawNode`, and `Metric`
   have no producer anywhere in `src/`; an agent filtering on them gets zero rows and
   reads that as “none present”. Delete them, or implement the raw-tree escape hatch
   if you want it.
5. **`report` carries a partial-index flag and the skip list**, so no count is ever
   presented as complete when it is not.
6. **Config precedence.** `ProjectConfig::apply_to_limits` rebinds the whole struct
   when a project `profile` is set, discarding every `SLUGAUDIT_*` env override
   already applied; unparseable env values silently fall back to defaults. Make the
   precedence explicit and layer rather than replace; warn on unparseable input.
   Files: `src/project/config.rs`, `src/model/limits.rs`.

### New enforcement

Extend `check_docs_drift` or add `check_schema_state` to assert that every
`EvidenceKind` variant has at least one producer and every `ExtractionCompleteness`
variant is reachable — cheap, and it prevents this class from returning.

### Acceptance

Tests: skip-aware resolution; existence row for a skipped file; `ContentOnly` for
unparsed-but-present content; kind-producer gate; partial flag in `report`; config
layering. **Size:** M/L. **Depends on:** nothing (can run parallel to Phase 3–5).

---

## Phase 3 — Governance shell

**Goal:** a reviewer can find out who owns what, how to contribute, and how to report
a vulnerability. All of these are absent today (verified: `.github/` holds only the
two workflows; no `OWNERS`/`CODEOWNERS`, `CONTRIBUTING*`, `SECURITY*`,
`CODE_OF_CONDUCT*`).

### Work

1. `OWNERS` / `.github/CODEOWNERS` mapping at least: `/src/sync/`, `/src/store/`,
   `/src/tools/`, `/.planning/ARCHITECTURE.md`, `/.github/`, `/AGENTS.md`.
2. `CONTRIBUTING.md` — build/test/gate commands, house style, branch and commit rules
   (unique subjects), DCO sign-off requirement, where plans live.
3. `SECURITY.md` — threat model (lazy/unsupervised agent, **not** a hostile attacker),
   what the tool can read, DB location and permissions, disclosure address, supported
   versions.
4. `CODE_OF_CONDUCT.md` — adopt Contributor Covenant verbatim.
5. `CHANGELOG.md` stub with the current version (Phase 4 wires enforcement).

### New enforcement

`check_governance` — a pure-Rust bin that fails when a required file is missing, or
when `CODEOWNERS` references a path that does not exist (same spirit as
`check_docs_drift`).

### Acceptance

Gate green in `quality.yml`; module map and Cargo.toml updated. **Size:** S.

---

## Phase 4 — Release, versioning, and supply chain

**Goal:** a released artifact is versioned once, attested, bill-of-materials'd, and
revertible.

### Work

1. **`check_version_consistency` gate.** The version is hand-duplicated today
   (`Cargo.toml` `1.0.10` and `lhm.plugin.json` `1.0.10`) with nothing tying them
   together. Assert equality across all manifests, and that the newest
   `CHANGELOG.md` entry matches, and that `parse::PACK_VERSION` matches its exact
   manifest pin (currently a unit test — fold it in).
2. **CHANGELOG discipline.** Either adopt release-please or add a gate requiring a
   `CHANGELOG.md` entry for the version being tagged.
3. **Rollback.** `slugaudit update` replaces the binary atomically but keeps no
   previous copy. Retain `slugaudit.prev` and document/implement an explicit revert
   path.
   Files: `src/update.rs`, `src/install.rs`, `install.sh`.
4. **Reproducibility.** Add `--remap-path-prefix` to the release build and a CI job
   that rebuilds a tag and compares hashes (best-effort, documented as such).
5. **SBOM.** Generate CycloneDX in `release.yml` and attach it to the release.
6. **Provenance / signing.** `actions/attest-build-provenance` (OIDC) at minimum;
   document how a user verifies it. Note the current gap explicitly: `SHA256SUMS` is
   fetched from the same origin as the artifact, which is integrity, not authenticity.
7. **Third-party notices.** Generate `THIRD_PARTY_LICENSES`/`NOTICE` from the
   dependency graph (`cargo-about` or `cargo-deny` output) and ship it with releases.
8. **PR-time supply chain.** Dependency review action, CodeQL or OSV-Scanner for
   Rust, and secret scanning (gitleaks) with `.planning/slugaudit/` excluded.

### Acceptance

A release dry-run produces: binary + `SHA256SUMS` + SBOM + notices + attestation.
New gates visible in CI. **Size:** M. **Depends on:** Phase 3 (SECURITY.md referenced
by notices/disclosure).

---

## Phase 5 — Untrusted-input rigor

**Goal:** the two surfaces where an agent feeds SlugAudit input are fuzzed and
adversarially tested.

`structure` executes agent-supplied Tree-sitter queries; `query` executes agent SQL
under a read-only connection, a subquery wrapper, and an authorizer hook. These are
the most interesting surfaces in the threat model and neither has a fuzz target.
Mutation testing covers four files, Linux-only.

### Work

1. **`fuzz/` crate (cargo-fuzz)** with three targets: (a) Tree-sitter query strings
   fed to `structure`, (b) SQL fed to `query`, including authorizer-bypass attempts,
   (c) JSON-RPC frame parsing. Seed corpora from `examples/queries/*` and
   `src/store/schema.sql`.
2. **Adversarial authorizer tests as permanent unit tests**: `ATTACH`, `PRAGMA`,
   `load_extension`, writes through CTEs and subqueries, comment/`;` splicing,
   multiple statements. Assert rejection, not just “no rows”.
   Files: `src/tools/query_tests.rs` + sibling `query_adversarial_tests.rs`.
3. **Widen mutation testing** beyond `sync/revision.rs`, `sync/publish.rs`,
   `sync/hash.rs`, `tools/context.rs` — or document why the narrower scope is
   intentional. A four-file gate implies coverage it does not have.
4. **Soak / large-repo test.** The largest bench fixture is 200 files; nothing
   exercises a 10k-file import except the wall-clock budget that would abort it.
   Add a synthetic large-repo test that stays inside `max_sync_wall_clock`.

### New enforcement

A **scheduled** (nightly, not per-PR) fuzz workflow. Per-PR keeps the fuzz targets
compiling only.

### Acceptance

Fuzz targets build and run clean for a bounded time; adversarial tests green; soak
test bounded. **Size:** M/L.

---

## Phase 6 — Shipped/dev artifact split

**Goal:** the product crate contains the product.

Today `Cargo.toml` declares six bins — one user-facing, five development gates — and
the repo's own `.planning/` development docs sit beside the runtime namespace.

### Work

1. Convert to a workspace: `crates/slugaudit` (product), `crates/slugaudit-gates`
   (the five `check_*` bins). Keep `Cargo.lock`, `deny.toml`, `rust-toolchain.toml`,
   and any `.cargo/config` at the root.
2. Delete `scripts/fast-check.sh`, or fold it into the gates crate with thresholds and
   annotation parsing **identical** to `check_source_limits`. It currently disagrees
   with the real gate on both, so a green fast-check proves nothing.
3. Update `release.yml` to build only the product crate.

### Coupling warning

This phase renames every path. It must land as one commit that also updates: the
ARCHITECTURE.md module map, the `*_tests.rs` count sentence, `include_str!` paths in
`src/resources.rs`, `check_docs_drift`'s expectations, and `check_source_limits`
counts. Do it **after** Phases 3–5 so their new files move once.

### Acceptance

`cargo build --release --bin slugaudit` produces an equivalent artifact; gate bins are
built only for dev/CI; full gate suite green. **Size:** L.

---

## Phase 7 — Interface contract and observability

**Goal:** the MCP surface is a versioned, testable contract, and failures are
branchable and correlatable.

### Work

1. **Pin the protocol version.** `src/server.rs` advertises
   `ProtocolVersion::LATEST`, which in `rmcp 3.1.4` resolves to `V_2025_11_25` and
   moves whenever the dependency is upgraded. Pin explicitly — `V_2025_06_18` matches
   the product's documented compatibility claim — and, if clients support it,
   advertise the supported range via discovery (`DiscoverResult.supported_versions`;
   constants available: `V_2024_11_05`, `V_2025_03_26`, `V_2025_06_18`,
   `V_2025_11_25`, `V_2026_07_28`).
2. **`check_mcp_surface` gate.** Snapshot `tools/list` + `resources/list` (names,
   descriptions, input **and output** schemas) to a committed JSON file and fail on
   drift, mirroring `check_docs_drift`. This is also what makes the next item
   contractual, so do them in the same phase: add `outputSchema` (response shapes),
   `annotations` (`readOnlyHint`/`destructiveHint`) and `title` to every tool, and
   publish a resource documenting per-kind `evidence.payload` shapes.
3. **Stable error codes.** Attach a machine-readable code to `ErrorData` alongside the
   human text (e.g. `data: {"code": "stale_revision"}`), document the code list, and
   keep the existing “often transient, retry” wording. Clients currently cannot branch
   on anything but prose.
4. **Call correlation.** Give each tool call an id in the tracing span inside
   `src/server_runner.rs` and emit it with every log line for that call.
5. **MSRV.** Declare `rust-version`, or document the deliberate policy that only the
   pinned toolchain is supported. “Not declared” reads as “not decided”.
6. **Ship the runbook.** `health`'s full-publish runbook currently points into
   `.planning/` (`PERFORMANCE.md`), which is development-only and would be
   customer-owned in a real project. Move it into `docs/`.

### Acceptance

Surface snapshot gate green; error codes documented and pinned by a test; runbook
present under `docs/`. **Size:** M. **Depends on:** Phase 6 (take the snapshot on
final paths/names).

---

## Not in scope

**Google requirements that do not transfer.** SLOs, on-call rotations, canary/gradual
rollout, staging environments, capacity planning, incident process, and a metrics
pipeline. A local stdio tool has no fleet to roll out to. The transferable analogue is
diagnosability — structured stderr logs, `SLUGAUDIT_LOG_FORMAT=json`, and the `health`
snapshot — which mostly exists and is completed in Phase 7.

**Product design work, tracked separately.** Span-addressed read tool
(`read_lines`/`read_span`), findings durability and export, the content-chunking
decision, and `leads`-vs-resources consolidation. These change what the product does;
this plan does not.

## Sequencing

```
Phase 1 ─┐
Phase 2 ─┼─ (code, independent)        → Phase 6 (moves everything) → Phase 7 (snapshot last)
Phase 3 ─┼─ (files + gate)
Phase 4 ─┤   depends on 3
Phase 5 ─┘   independent
```

Phases 1, 2, 3, and 5 are independent of each other and can run concurrently. Phase 4
depends on Phase 3 (disclosure doc). Phase 6 must follow 3–5 so their files move once.
Phase 7 must follow 6 so the interface snapshot is taken on final paths.

## Risk register

| Risk | Mitigation |
|---|---|
| Workspace split (6) invalidates the module map/docs gate | land as one commit with the doc updates; do it after 3–5 |
| Phase 2 changes `files` semantics | audit every consumer assuming `files` == indexed set; manifest hash must handle non-indexed rows |
| Phase 2 adds a schema version bump | free — mismatch path discards and rebuilds; no migration to write |
| 200-line cap bites when adding `skip_reason` plumbing | expect annotations or module splits; check `check_source_limits` per commit |
| Pinning the protocol to `V_2025_06_18` narrows compatibility with newest clients | advertise the range via discovery if the client supports it |
| Nightly fuzzing costs CI minutes | scheduled workflow, not per-PR; per-PR only compiles the targets |
| Every phase adds files | unique test names, unique commit subjects, module map updated same commit |
