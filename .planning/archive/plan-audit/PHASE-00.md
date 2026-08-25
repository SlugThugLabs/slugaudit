# Plan Audit — Phase 0

Status: audited

Scope: audit the plan and its acceptance criteria, not the Rust implementation.

## Verdict

FAIL. If Phase 0 is followed exactly, it can be marked complete while the
project is still a five-line placeholder binary with no MCP server, tests,
logging contract, executable source policy, or process error contract.

## Scores

| Area | Score | Reason |
|---|---:|---|
| Architecture | 4/10 | Ownership is described, but the modules do not exist and no dependency-direction check exists. |
| Maintainability | 4/10 | The 200-line rule is clear, but enforcement and contributor workflow are deferred. |
| Readability | 7/10 | The prose is understandable, but it is not yet tied to executable acceptance artifacts. |
| Performance | 1/10 | No startup budget or Phase 0 measurement exists. |
| Security | 4/10 | Unsafe and path concerns are named, but protocol, dependency, and redaction gates are deferred. |
| Testability | 2/10 | There is no test harness, fixture, or test in the new project. |
| Error checking | 1/10 | No meaningful error contract or failure-path tests exist. |
| Logging/tracing | 0/10 | No logging implementation or coverage exists. |

## Findings

### P0-01 — A successful non-product binary can pass the phase

Location: `src/main.rs:3-5`; Phase 0 Tasks 0.1–0.4.

`main()` only writes “not implemented yet” to stderr and exits successfully.
The plan does not require a minimal MCP handshake or a non-zero exit for this
state. A mechanical phase review could therefore mark a non-server artifact
green.

Severity: high.

Correction: add a Phase 0 executable baseline. It must either implement and
test a minimal MCP initialize response or exit non-zero with a tested,
explicitly incomplete status. The phase gate must reject a successful
placeholder process.

### P0-02 — The architecture is only a directory diagram

Location: `ARCHITECTURE.md`; plan section 3; absent `src/server`, `src/tools`,
`src/model`, `src/store`, and the other listed modules.

The documentation can drift without a compile-time ownership boundary.

Severity: medium.

Correction: add a module skeleton task and a compile-time architecture check.
The check must reject prohibited dependencies, such as tools importing parser
internals or parser code importing SQLite.

### P0-03 — The quality bar is enforced too late

Location: Phase 0 Task 0.4 and Phase 11 Task 11.1.

CI is deferred until Phase 11, allowing ten phases to violate the core rules.
The no-unsafe and sub-200-line rules need to be enforced from the first phase.

Severity: high.

Correction: add a local structural gate and minimal CI in Phase 0. Phase 11 may
expand CI but must not be the first enforcement point.

### P0-04 — No test harness exists before feature work

Location: Phase 0 as a whole.

The plan later asks for extensive tests but does not first prove that tests,
fixtures, and nextest discovery work.

Severity: high.

Correction: add a baseline unit test, integration test, fixture project, and
`cargo nextest` invocation to Phase 0.

### P0-05 — Process error behavior is unspecified

Location: `src/main.rs`; Phase 0 Tasks 0.1–0.4.

There is no typed top-level error, exit-code policy, panic policy, or boundary
between startup errors and protocol errors.

Severity: high.

Correction: define process-level `Result`, exit codes, panic containment, and
the rule that internal details go to stderr while protocol errors are returned
through MCP.

### P0-06 — Logging/tracing is absent from the baseline

Location: no logging module; no Phase 0 logging task.

The later `src/runtime/logging.rs` task does not define early requirements for
stderr-only output, sensitive-value redaction, correlation IDs, startup events,
or log levels. “Proper logging” would remain subjective.

Severity: high.

Correction: add a Phase 0 logging contract and stdout-purity test. Require
startup, error, and request-boundary events before Phase 1.

### P0-07 — Applicable user-experience risks are missing

Location: Phase 0; performance first appears in Phase 9.

This is an MCP stdio service, not a TUI: rendering bugs and keybinding
conflicts are not applicable. The applicable risks are unplanned: blocking
parser downloads at startup, no feedback during a long first sync, and no
visible distinction between startup, syncing, and querying.

Severity: medium.

Correction: mark TUI/keybinding checks not applicable and add startup latency,
first-sync progress, stderr progress events, and MCP-safe progress behavior to
the early gates.

### P0-08 — Dependency/native-code policy starts too late

Location: `Cargo.toml`; Phase 0; Phase 11 Task 11.2.

Tree-Sitter and bundled SQLite introduce native/FFI behavior. The plan permits
transitive unsafe, correctly, but does not require a dependency inventory,
license policy, or geiger baseline before adding product code.

Severity: medium.

Correction: move initial `cargo deny` configuration, `cargo audit`, and
`cargo geiger` baseline into Phase 0.

### P0-09 — Plan changes have no decision trail

Location: Phase 0 and phase-completion rules.

The plan can be weakened during implementation without recording why a gate
changed or what replaces it.

Severity: medium.

Correction: add a dated decision log. Every changed acceptance criterion must
state reason, impact, and replacement validation.

### P0-10 — “Production quality” is not measurable enough

Location: `ARCHITECTURE.md`; plan introduction.

The plan uses senior-review language but does not yet require evidence for
dependency direction, startup behavior, operational failure, onboarding, or
observability.

Severity: medium.

Correction: replace broad quality claims with measurable gates and a review
checklist covering architecture, reliability, observability, security,
performance, and testability.

## Code-smell audit

There are not ten real production functions yet. Inventing ten would be
dishonest. The confirmed smells are:

1. `src/main.rs::main` — successful placeholder process; high.
2. `src/main.rs::main` — no returned error path; high once startup exists.
3. `src/main.rs::main` — hard-coded status instead of a lifecycle contract;
   medium.
4. `ARCHITECTURE.md` module map — documented modules do not exist; medium.
5. `Cargo.toml` — dependencies exist before a use/ownership policy; medium.
6. Plan Phase 0 — standards are specified before executable enforcement; high.
7. Plan Phase 0 — no baseline test harness; high.
8. Plan Phase 0 — no startup/error/logging contract; high.
9. Plan Phase 0 — no long-operation feedback contract; medium.
10. Plan Phase 0 — no decision log for changing gates; medium.

Items 4–10 are plan smells rather than Rust smells, but they are production
risks because this plan is the implementation specification.

## Complexity

Current production function ranking:

1. `src/main.rs::main` — cyclomatic complexity 1; trivial placeholder.

There are no other production functions. This does not prove the final design
will be simple. The plan must require a complexity report after each phase,
with a threshold and written justification for unavoidable state machines.

## Testing

Realistic current product-logic coverage: 0%. The project has no tests.

Untested critical paths:

- startup and process failure;
- MCP handshake and stdout purity;
- stderr logging;
- root/path validation;
- parser/dependency loading;
- SQLite open and rollback;
- source-size/no-unsafe gates;
- architecture ownership;
- future tool requests.

Correction: Phase 0 must add baseline tests for all applicable paths. Later
coverage percentages must come from `cargo llvm-cov`, not estimates.

## Error checking

Realistic current coverage: 0% of product behavior. No error contract exists.

Correction: add tests for invalid root, unsupported startup state, malformed
protocol input, controlled startup failure, panic containment, and redacted
internal errors. Require each later public operation to return a typed `Result`
and maintain a failure-mode-to-test matrix.

## Logging/tracing

Realistic current coverage: 0%.

Missing paths:

- startup begin/end/failure;
- tool received/completed/failed;
- sync begin/summary/failure;
- parser cache hit/download/failure;
- SQLite rollback;
- stale-revision rejection;
- protocol decode failure;
- unexpected panic boundary.

Correction: require stderr-only structured logs, redaction, levels, request IDs,
and stdout-purity testing in Phase 0. Phase 3, 4, and 7 must each test their
own operational events.

## Security

Concrete plan risks:

- Future logs may corrupt MCP framing because no stdout test exists.
- Dependencies may be added without advisory/license review because policy is
  deferred.
- Future handlers may trust traversal or absolute paths because validation is
  not yet implemented.
- A successful placeholder process could appear healthy to an MCP host.

There is no functioning-server exploit yet because there is no server. These
are gaps in the plan that could become vulnerabilities if uncorrected.

## Maintainability

A new developer is slowed by the absence of:

- README/build/run instructions;
- contributor workflow;
- module skeleton;
- fixture conventions;
- error/logging conventions;
- direct versus transitive unsafe policy;
- decision log.

Correction: make these Phase 0 deliverables, not documentation work deferred
until handoff.

## Required plan corrections before Phase 1

Add these Phase 0 tasks:

1. Create `README.md` with build, test, run, and project-boundary instructions.
2. Create the initial module skeleton matching `ARCHITECTURE.md`.
3. Create a baseline test harness and fixture project.
4. Create and execute `tools/check_source_limits.sh`.
5. Add minimal CI for format, check, clippy, tests, file size, and no unsafe.
6. Define process `Result`, exit codes, panic handling, and error redaction.
7. Define stderr-only structured logging and stdout-purity tests.
8. Define startup latency and first-sync progress budgets.
9. Add dependency/license/unsafe inventory.
10. Add a dated plan decision log.
11. Add a complexity baseline report.
12. Mark TUI rendering/keybindings not applicable while retaining protocol UX,
    startup, and progress checks.

## Phase 0 decision

FAIL. Do not begin Phase 1 until these corrections are added to the master plan
and their validations are executed. This note records the findings; the master
plan still needs to be corrected after all phase audits are complete.
