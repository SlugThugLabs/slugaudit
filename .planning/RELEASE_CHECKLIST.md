# Release checklist

Purpose: the complete, runnable release gate for SlugAudit. Every item must
exit zero and every artifact must be recorded before a release is
considered complete (Task 12.3 + Phase 13 + §20 acceptance criteria).
Skipped or failed items must be reported with a reason — they must never be
described as passing.

## 0. Pre-flight state

- [ ] Working tree reviewed: `git status --short` shows only intended changes.
- [ ] `git diff --check` is clean.
- [ ] Active toolchain is the pinned compiler:
      `rustup show active-toolchain` reports `1.97.1`.
- [ ] `rust-toolchain.toml` is unchanged from the pinned compiler/edition.
- [ ] `Cargo.lock` is checked in and consistent: `cargo metadata --locked`
      succeeds.
- [ ] No new dependency was added without a matching `.planning/DEPENDENCIES.md`
      entry and a dated decision-log entry (Task 11.2).
- [ ] No production `.rs` file grew past 200 code lines without a documented
      `slugaudit-line-exception` (run `tools/check_source_limits.sh`).
- [ ] No source-limit, unsafe, or gate policy was weakened to make a change
      pass.

## 1. Complete release gate

Run from `/opt/slugaudit-mcp-rust`:

```bash
rustup show active-toolchain
cargo fmt --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-targets --locked
cargo test --workspace --doc
cargo llvm-cov nextest --workspace --all-features --lcov --output-path lcov.info
cargo audit
cargo deny check advisories bans sources licenses
cargo geiger --all-features
tools/check_source_limits.sh
git diff --check
```

Record in the release notes:

- [ ] Coverage percentage from `cargo llvm-cov report --summary-only`
      (gate: ≥ 89% line coverage; the CI gate recalibrates deliberately,
      not aspirationally).
- [ ] `cargo audit` result (zero known vulnerabilities expected).
- [ ] `cargo deny` result and license allow-list state.
- [ ] `cargo geiger` inventory reviewed; any new transitive unsafe named in
      `.planning/DEPENDENCIES.md`.
- [ ] Source-limit output (every production file under 200 code lines, or an
      approved exception listed).

## 2. Correctness-surface checks (not optional)

- [ ] Mutation testing on the CAS/retry/hash/freshness surface
      (`src/sync/revision.rs`, `src/sync/publish.rs`, `src/sync/hash.rs`,
      `src/tools/context.rs`): zero surviving mutants, or every survivor
      has a dated review proving it is not meaningful behavior
      (amendment 21.9). CI runs this `continue-on-error`; a release must
      not rely on that.
- [ ] Real stdio protocol test passes: `cargo test --test stdio_protocol --locked`
      (asserts stdout is protocol-pure and stderr carries the documented
      event fields).
- [ ] Adversarial/race suites pass: `cargo nextest run --all-targets`
      includes the publish-race, finding-race, and context-race suites.
- [ ] Watcher-backed incremental sync is exercised (dirty-path reconcile
      path, not only full publish).

## 3. Documentation consistency (Phase 13)

- [ ] `README.md` build/test/run instructions are accurate from a clean
      temporary checkout.
- [ ] `ARCHITECTURE.md` module map matches the filesystem (every listed
      file exists; `*_tests.rs` count matches).
- [ ] `OBSERVABILITY.md` matches current tracing/operational behavior.
- [ ] `.planning/README.md` "Current state" matches the code being released.
- [x] `.planning/PERFORMANCE.md` exists and records the baseline for this
      release (Task 9.2; recorded 2026-08-10 — see decision log).
- [ ] No documentation claims behavior the tool does not provide ("risk
      leads", "automatic bugs", "audit completed", manual sync commands).
- [ ] `.planning/RELEASE_CHECKLIST.md` itself is current.

## 4. End-to-end acceptance (Task 12.2 / §20)

- [x] The multi-language fixture with versioned golden manifest and
      evidence contract exists and is asserted by `tests/fixture_contract.rs`
      (Task 12.1, recorded 2026-08-10 — see decision log).
- [ ] The fixture repository workflow runs against the real binary:
      activate → server start → `initialize` → `report` → `query` (symbol,
      source span, recursive-CTE dependency traversal, parser diagnostics)
      → `structure` pattern match → write-through-`query` rejected at the
      connection level → `finding` persisted → source modified → stale
      finding + new revision verified.
      (**Pending:** the real-MCP workflow test — Task 12.2 — see decision
      log.)
- [ ] All responses carry verified freshness metadata.
- [ ] All evidence responses are bounded (limits metadata present where
      applicable).
- [ ] No response contains automated risk leads or an audit verdict.
- [ ] stdout contains only MCP frames; stderr contains operational logs.
- [ ] Restart behavior: stop the server, reopen the database, and confirm
      the project's last revision is served and freshness re-verification
      works.

## 5. Release artifacts

Record in the release notes or an attached artifact manifest:

- [ ] Project license: `PolyForm-Noncommercial-1.0.0` (in `Cargo.toml` and
      `deny.toml` allow-list).
- [ ] Third-party attribution list (generated from `cargo about` or the
      license inventory in `.planning/DEPENDENCIES.md`).
- [ ] Pinned lockfile (`Cargo.lock`), compiler identity (`1.97.1`), edition
      (`2024`).
- [ ] Build metadata and reproducible artifact checksum
      (e.g. `cargo build --release --locked` + `sha256sum` of the binary;
      record the value).
- [ ] Dated decision-log entries for every exception or deviation taken
      during this release cycle.

## 6. Post-release

- [ ] Tag the release commit.
- [ ] Update `.planning/README.md` "Status" and any dated artifacts to
      reflect the release.
- [ ] Confirm `/opt/slugaudit-mcp` (the old Python checkout) can be deleted
      without affecting this build (§20 final criterion).
