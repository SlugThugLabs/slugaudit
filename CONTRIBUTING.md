# Contributing to SlugAudit

This file is a signpost, not a second copy of the house rules. Long-form
guidance lives in [`AGENTS.md`](AGENTS.md); the design and the invariants live
in [`.planning/ARCHITECTURE.md`](.planning/ARCHITECTURE.md). Read both before
changing anything under `src/`, and treat them as authoritative over anything
summarised here. Duplicated guidance drifts; pointers do not.

## Toolchain

Rust **1.97.1**, edition 2024, pinned in `rust-toolchain.toml` (minimal
profile + rustfmt + clippy). There is no declared MSRV — do not invent one.
Toolchain and lockfile updates are their own deliberate change, never a side
effect of unrelated work.

## Build and test

```bash
cargo build --release --locked            # the shipped binary
cargo run                                 # serve MCP over stdio
cargo test --lib --bins --tests --all-features --locked   # CI bulk run
cargo test --lib <name_substring>         # one test
cargo test --test stdio_protocol --locked # real JSON-RPC over stdio pipes
```

## Gates

These are the checks CI runs, in order (`.github/workflows/quality.yml`,
Linux and macOS). Run them before pushing.

```bash
cargo fmt --all -- --check
cargo run --quiet --bin check_source_limits --locked      # LoC ceilings + exception annotations
cargo run --quiet --bin check_no_duplicates --locked      # duplicate commit subjects / test fn names
cargo run --quiet --bin check_docs_drift --locked         # ARCHITECTURE module map, test count, canonical docs
cargo run --quiet --bin check_producer_coverage --locked  # every evidence kind is constructed somewhere
cargo run --quiet --bin check_governance --locked         # governance files exist; CODEOWNERS paths resolve
cargo run --quiet --bin check_version_consistency --locked # Cargo.toml = lhm.plugin.json = CHANGELOG = PACK_VERSION
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --lib --bins --tests --all-features --locked
cargo test --test stdio_protocol --locked
cargo build --release --locked
```

Then CI runs `cargo-deny` (advisories, bans, sources, licences — its `advisories`
check is the RustSec advisory database), `check_coverage` (83% line coverage by
default), and — Linux only — `cargo mutants` over the revision/publish/hash/context
surface. Two further workflows cover what a push cannot:
`.github/workflows/advisories.yml` re-runs the `deny.toml` advisory policy weekly,
and `.github/workflows/supply-chain.yml` gates each pull request on dependency
review, an OSV.dev scan of `Cargo.lock`, and a secret scan. A new dependency must
clear `cargo deny`.

`check_coverage` requires `cargo-llvm-cov`. `check_performance` is a **local** gate
and is deliberately in no workflow: its baseline records the machine that measured
it, and criterion medians on a shared, unpinned runner differ from that machine for
reasons unrelated to the commit, so the allowance would measure the runner. Run both
locally:

```bash
cargo run --quiet --bin check_coverage --locked
cargo run --bin check_performance --locked              # criterion medians vs baseline, 20%
cargo run --bin check_performance --locked -- --record  # deliberately re-baseline
```

What the gates mean, and the exception annotations they accept, are documented
in `AGENTS.md` (*Code conventions* → *Source-size gate*, *Doc-drift coupling*).

## House style

One entry point: [`AGENTS.md`](AGENTS.md). It carries the lint contract, the
error-handling rules, the tool-extension recipe, the test layout and naming
rules, and the per-surface conventions. `.planning/ARCHITECTURE.md` carries the
numbered invariants — a change that breaks one is a bug even if tests pass.

Two rules bite most often:

- **No silent truncation.** Every cap, skip, and producer gap is reported in
  the response or it is a defect.
- **Every count must be quotable as-is.** Evidence is not judgment: no scores,
  no severity, no ranking.

## Branches

Not yet defined. No branch-naming rule is enforced by CI and the repository
does not currently document one; pick a descriptive name and keep the branch
scoped to one logical change. If a convention is adopted, record it here rather
than leaving it in a session.

## Commits

Commit subjects must be **globally unique** across reachable history.
`check_no_duplicates` fails on a repeated subject — a repeat is the fingerprint
of the same logical change committed twice. The gate runs with full history
(`fetch-depth: 0` in `quality.yml`), so a subject already used anywhere in the
branch's history is unavailable, and rewriting history changes which subjects
exist. Test function names are held to the same uniqueness rule across `src/`
and `tests/`.

Write the subject in the imperative mood and scope it when useful, in the style
already in `git log`:

```
fix(project): create the index directory owner-only, and pin the sidecar modes
docs(plan): mark the offset and char_column follow-ups resolved
```

## Sign-off (DCO)

Every commit must carry a `Signed-off-by:` trailer certifying the
[Developer Certificate of Origin](https://developercertificate.org/):

```bash
git commit -s -m "fix(sync): ..."
```

`git commit -s` appends the trailer from your `user.name`/`user.email`. This is
a project requirement recorded in `.planning/PLAN.md`; it is **not yet
machine-enforced** — no CI job checks the trailer today, so keep it by hand.
Add this as a gate before relying on it.

## Where plans and decisions live

- [`.planning/PLAN.md`](.planning/PLAN.md) — the single active plan. Bugs and
  gaps go here, as a phase or in the follow-ups list.
- [`.planning/archive/DECISIONS.md`](.planning/archive/DECISIONS.md) —
  append-only decision log. Add entries; never rewrite history.
- [`.planning/ARCHITECTURE.md`](.planning/ARCHITECTURE.md) — design, module
  map, invariants.
- `docs/TROUBLESHOOTING.md` — the operator runbook.

If a decision, a discovery, or a rejection exists only in a conversation, it is
lost. `AGENTS.md` (*Session hygiene*) states the rule: write it down in the
same turn, in the document a future reader will open, with an explicit state
(`landed`, `not started`, `still open`, `dropped because …`) and a verifiable
form — a date, a version, a path, a command, a `path:line`.

## Review

[`.github/CODEOWNERS`](.github/CODEOWNERS) routes review to the SlugThugLabs
organisation. Ownership covers the whole tree; the indexing, store, and tool
surfaces are called out explicitly.

## Security and conduct

Do not open a public issue for an unfixed vulnerability — see
[`SECURITY.md`](SECURITY.md). Participation is covered by
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Licence

SlugAudit is licensed under the PolyForm Noncommercial License 1.0.0 plus the
SlugAudit Tool-Use Additional Permission, carried in [`LICENSE`](LICENSE) —
non-SPDX, hence `license-file` and `publish = false` in `Cargo.toml`. The
additional permission covers running and modifying SlugAudit as an internal
development tool; distributing or embedding SlugAudit itself does not.

The repository does **not** yet state an inbound contribution licence (no DCO
clause beyond the sign-off above, no CLA, no licence header rule). Do not
assume one; if you need it settled before contributing, ask first rather than
reading an assurance into this file.
