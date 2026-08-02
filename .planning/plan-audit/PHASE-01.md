# Plan Audit — Phase 1

Status: audited

Scope: typed evidence contract only; no Rust implementation audit.

## Verdict

FAIL as written. Phase 1 has the right subject area, but its types are
underspecified and combine states that must remain separate. If followed
literally, later storage and MCP layers would likely persist ambiguous parser
states, unstable JSON, unbounded raw output, and evidence records that cannot
explain missing spans.

## Phase score

| Area | Score | Reason |
|---|---:|---|
| Architecture | 5/10 | Model ownership is identified, but lifecycle, evidence, and response concerns overlap. |
| Maintainability | 4/10 | File names suggest separation, but the contracts do not define identity, ordering, or versioning. |
| Readability | 6/10 | The type list is understandable; important semantics remain implicit. |
| Performance | 3/10 | Raw pack JSON is optional/unbounded and no model-size budget is defined. |
| Security | 5/10 | Path/hash concepts exist, but parser/cache/error metadata and raw evidence limits are not specified. |
| Testability | 5/10 | Several unit tests are named, but property, boundary, and compatibility tests are missing. |
| Error checking | 4/10 | Parser failure is named, but lifecycle and extraction failure states are not orthogonal. |
| Logging/tracing | 1/10 | The model has no operational event/correlation contract. |

## Findings

### P1-01 — `ParserStatus` conflates three different state machines

Location: Phase 1 Task 1.2, `src/model/parser.rs`.

The proposed states `unavailable`, `downloaded`, `loaded`, `parsed`, and
`failed` are not one valid enum. `downloaded` and `loaded` describe parser
resource lifecycle; `parsed` and `failed` describe a file operation. A parser
can be loaded while parsing fails, and a parser can be available without being
downloaded by this process. A single enum forces impossible or ambiguous
states.

Severity: high.

Correction: define separate types:

- `ParserAvailability`: catalog-missing, available-cached, downloaded,
  unavailable, load-failed;
- `ParseOutcome`: not-attempted, succeeded, syntax-errors, failed;
- `ExtractionCompleteness`: full, partial, content-only, unavailable;
- `EvidenceOrigin`: pack-structure, pack-symbol, raw-tree, source-content,
  derived-relationship.

Validation must test the legal combinations and reject impossible combinations
such as `ParseOutcome::Succeeded` with `ParserAvailability::Unavailable`.

### P1-02 — Source identity and parser state are mixed responsibilities

Location: Task 1.1 `src/model/source.rs`; Task 1.2 `src/model/parser.rs`.

Path, content identity, file metadata, language selection, parser resource
state, and parse result are distinct concerns. If `SourceFile` owns all of them,
every change to parsing or filesystem metadata changes the same type and
encourages coupling between sync, parser, and store layers.

Severity: medium.

Correction: separate `SourceIdentity`, `FileMetadata`, `LanguageSelection`,
`ParserRun`, and `FileEvidence`. Compose them in a file snapshot rather than
making one universal record.

### P1-03 — Span semantics are not precise enough for trustworthy evidence

Location: Task 1.1.

The plan says “byte span” and “one-based line and column span” but does not
define whether the end is inclusive or exclusive, which encoding columns use,
or how invalid UTF-8 is handled. A code auditor can be misdirected by an
off-by-one or byte/character mismatch.

Severity: high.

Correction: define byte ranges as zero-based, half-open; define displayed lines
as one-based; define columns as UTF-8 byte offsets or explicitly provide both
byte and Unicode scalar columns; define behavior for invalid UTF-8.

Validation must include empty spans, EOF spans, Unicode before a match,
multi-byte characters, CRLF, and invalid UTF-8.

### P1-04 — Evidence identity and deduplication are unspecified

Location: Task 1.2.

No stable key is defined for a definition, symbol, import, diagnostic, comment,
or chunk. Without one, repeated parser captures or normalization changes can
create duplicates, unstable query ordering, and noisy AI context.

Severity: high.

Correction: define an evidence key from file hash, evidence kind, source span,
name/value identity, and normalized payload version. Define deterministic sort
order and deduplication rules.

### P1-05 — “Raw pack JSON where practical” is not an acceptance criterion

Location: Task 1.2 rule 5.

This phrase permits arbitrary storage growth and inconsistent evidence across
languages. Raw output can duplicate source content, expose implementation
details that the AI cannot use, and make schema changes expensive.

Severity: high.

Correction: define a bounded raw-evidence policy: what is retained, maximum
bytes per file/record, compression or omission behavior, pack-version metadata,
and truncation flags. “Where practical” must not remain a production rule.

### P1-06 — Missing-span evidence has no explanation contract

Location: Task 1.2 rule 1.

“When the pack provides one” is honest but incomplete. A record without a span
could mean the pack omitted it, normalization lost it, or the record is derived.
Those have different trust implications.

Severity: high.

Correction: require `SpanAvailability` with `Present`, `PackOmitted`,
`NormalizerUnavailable`, or `DerivedEvidence`, plus a reason field where
needed.

### P1-07 — Hash algorithm and canonicalization are unspecified

Location: Task 1.1.

The plan says “content hash” but not algorithm, encoding, or whether hashing is
over raw bytes or normalized text. This can break stale-finding invalidation and
cross-run freshness.

Severity: high.

Correction: define a named algorithm and hash raw file bytes. Define lowercase
hex or binary storage, and include algorithm/version in the identity format.

### P1-08 — Timestamp semantics are unspecified

Location: Task 1.1 and Task 1.3.

“Modification timestamp when available” is not enough for deterministic
revisions. It does not define timezone, precision, missing-value behavior, or
whether timestamps are advisory only.

Severity: medium.

Correction: use an explicit UTC representation, preserve optionality, and state
that content hash—not mtime—controls correctness.

### P1-09 — Freshness metadata does not define proof requirements

Location: Task 1.3.

The plan names contract/schema/manifest/parser versions but does not define
which exact inputs are hashed, how a revision is verified, or whether every
response wrapper can enforce metadata presence.

Severity: high.

Correction: define a `VerifiedRevision` capability/type that can only be created
after manifest comparison. Require public response constructors to accept it,
not an arbitrary metadata struct.

### P1-10 — The plan does not record the pack feature/API contract

Location: `Cargo.toml`; Task 1.2.

The current dependency has default features enabled, and the current crate
documentation lists serialization support among the default download features.
So the earlier concern that serde is definitely missing is not confirmed. The
real plan defect is that the feature assumption is undocumented and untested.
The plan also does not distinguish the pack’s high-level `process()` API from
the low-level parser API.

Severity: high.

Correction: record the exact pack version and enabled features in the
dependency note, compile an API probe, and map pack types into SlugAudit-owned
types instead of making the pack’s serialization format the SlugAudit public
contract. If default features change, the probe must fail visibly.

### P1-11 — Serialization tests alone will freeze unstable external details

Location: Task 1.2 validation 1 and Task 1.3 snapshot test.

“Serialize and deserialize every evidence type” can create a false sense of
compatibility and snapshots may lock external enum spelling or field layout.
The public contract needs deliberate wire-format tests, not blanket snapshots.

Severity: medium.

Correction: define an explicit public JSON schema owned by SlugAudit, map pack
types into it, and snapshot only the public contract. Add unknown-field and
version tests.

### P1-12 — Model limits are absent despite token-efficiency being core product
behavior

Location: Phase 1 as a whole.

There are no maximum counts or bytes for definitions, symbols, diagnostics,
raw nodes, chunks, or serialized file evidence. Later tool limits cannot fully
repair an oversized database or expensive normalization step.

Severity: high.

Correction: add per-file and per-response budgets to the model contract, with
explicit truncation and completeness fields.

## Code-smell audit of the plan

No Phase 1 production functions exist yet, so function-level cyclomatic smells
cannot honestly be listed. The ten worst design smells are:

1. `ParserStatus` — invalid mixed state machine; critical.
2. `SourceFile` concept — likely god-record; high.
3. `raw pack JSON where practical` — undefined data-retention policy; high.
4. `Every record carries a span when available` — missing provenance state; high.
5. `content hash` — unspecified algorithm/canonicalization; high.
6. `modification timestamp when available` — unspecified semantics; medium.
7. Evidence records — no stable identity/dedupe key; high.
8. Public responses — metadata presence not type-enforced; high.
9. Pack API/features — undocumented dependency contract; high.
10. Snapshot validation — external wire format could become accidental contract;
    medium.

## Verified language-pack facts that change the plan

The current crate documentation confirms the following and these facts must be
treated as implementation constraints:

- The crate advertises 306 language parsers, not merely the old eight-language
  set.
- Parsers are downloaded on first use and cached for reuse. Cache location,
  offline behavior, cache corruption, and first-use latency are product
  behavior, not incidental implementation details.
- `process()` is the intended high-level structured-analysis API. It returns
  structure, imports, exports, comments, docstrings, symbols, diagnostics,
  metrics, and chunks according to `ProcessConfig`.
- The pack does not expose an arbitrary query-execution API through
  `process()`. SlugAudit must not plan to send arbitrary Tree-sitter queries
  through that function.
- The low-level `get_parser()` API remains appropriate for raw syntax-tree
  evidence and manual walks when the normalized result does not provide enough
  detail.
- The pack recommends reusing parser objects for batch processing of one
  language. Parallel workers should therefore reuse a parser per worker or
  language context rather than construct one for every file.
- Canonical language names are lowercase and case-sensitive, with aliases.
  Detection and freshness must normalize aliases before persistence.
- Tree-sitter recovers from malformed syntax. Diagnostics and an error-bearing
  partial tree are evidence; they are not automatically a total parser failure.
- The pack’s `symbols` output is a deduplicated identifier list. It must not be
  described as complete reference resolution.
- Structure kinds vary by language. SlugAudit must preserve unknown kinds
  instead of claiming a universal semantic taxonomy.
- The upstream repository is `xberg-io/tree-sitter-language-pack`, and the
  crate documentation links to it. The plan should cite this repository as the
  source of truth for catalog, query coverage, ABI, cache, and release facts.
- The upstream language table shows that bundled query coverage varies by
  grammar. Not every language has tags, locals, injections, indents, folds, or
  other query files. SlugAudit must report capability per language rather than
  treating all 306 parsers as equally rich.
- The repository advertises ABI 14 and ABI 15 grammars and compatibility across
  Tree-sitter versions. The plan must validate the actual catalog against the
  pinned Tree-sitter runtime instead of assuming every grammar loads identically.
- The upstream repository includes `prefetch`/warming behavior. The plan should
  decide whether SlugAudit warms only detected project languages or downloads
  on demand during sync; silently downloading all 306 grammars would be a bad
  startup and disk policy.

Validation additions required by these facts:

1. Compile an API probe against the pinned Rust crate and default features.
2. Assert the language catalog count and canonical/alias normalization.
3. Exercise cold-cache, warm-cache, offline, and corrupted-cache behavior.
4. Compare `process()` output with low-level `get_parser()` output for a
   fixture where raw spans or node information are needed.
5. Verify diagnostics are persisted alongside partial structure.
6. Verify symbols are labeled as identifiers, not references.
7. Verify one parser is reused for a batch and output remains deterministic.
8. Build a language capability matrix from the upstream catalog, including
   parser load, structure, diagnostics, tags/query availability, and data
   extraction.
9. Test ABI 14 and ABI 15 representatives against the pinned runtime.
10. Test prefetch/warm behavior separately from first-use on-demand loading.

The docs.rs page for the latest release currently reports that its hosted docs
build failed, even though the crate resolves and compiles in the new project.
That is not proof that the crate is broken, but it is a release-process risk.
The plan must include a local API probe and a pinned dependency build in CI
rather than relying on hosted documentation availability.

## Testing assessment

Current realistic coverage: 0% because no Phase 1 code or tests exist.

The plan’s named tests cover basic construction but miss:

- legal/illegal parser state combinations;
- half-open span boundaries;
- Unicode and invalid UTF-8;
- evidence deduplication and deterministic ordering;
- raw-evidence budgets and truncation;
- missing-span provenance;
- hash algorithm stability;
- timestamp precision/timezone;
- unknown public JSON fields and version changes;
- pack feature/API compilation;
- property-based generation of spans and evidence records.

Correction: add those cases to Phase 1’s validation and require coverage from
`cargo llvm-cov`, while separately testing contract invariants that line
coverage cannot prove.

## Error-checking assessment

Realistic current coverage: 0%.

Phase 1 needs typed errors for invalid paths, invalid spans, unsupported
language names, parser lifecycle contradictions, malformed pack output,
oversized evidence, serialization failure, and missing freshness proof.

Correction: add an error matrix mapping each error variant to a unit test and a
later MCP-facing behavior.

## Logging/tracing assessment

Realistic current coverage: 0%.

The model phase needs trace fields for file path/hash, language selection,
parser availability, parse outcome, evidence counts, truncation, and duration.
It must not log source content or secrets.

Correction: add an evidence-normalization event contract and redaction tests.

## Security and operational risks

- Unbounded raw evidence can create memory/storage denial of service on a large
  or generated file.
- Ambiguous spans can cause the AI to inspect the wrong source location.
- Unstable evidence keys can leave stale or duplicated findings.
- Missing parser provenance can make failed extraction look like absence.
- External pack serialization could expose more data than intended through the
  MCP response.
- On-demand parser downloads can block or fail during a first audit and can
  become a supply-chain/cache-integrity boundary. The plan needs checksums,
  cache permissions, and explicit offline behavior.
- Treating “306 languages” as “306 equally complete extraction profiles” can
  produce false completeness claims. The upstream capability matrix must be
  persisted and surfaced to the AI.

## Maintainability risks

A new developer will not know which types are authoritative, whether pack types
may cross module boundaries, or which JSON fields are stable. The plan must
declare SlugAudit-owned models as the only cross-layer contract and keep pack
types inside `parse/`.

## Required corrections to the master plan

Before Phase 2 begins, change Phase 1 to require:

1. Separate parser availability, parse outcome, and extraction completeness.
2. Separate source identity, file metadata, language selection, parser run, and
   file evidence.
3. Specify span units, indexing, end semantics, Unicode, CRLF, and invalid
   UTF-8 behavior.
4. Specify hash algorithm, raw-byte canonicalization, and encoding.
5. Specify timestamp representation and correctness role.
6. Define stable evidence IDs, deduplication, and ordering.
7. Define missing-span provenance.
8. Define bounded raw-evidence retention and truncation fields.
9. Define per-file/per-response evidence budgets.
10. Make verified freshness a capability/type, not a freely constructible
    response field.
11. Keep language-pack types inside the parser adapter and define an explicit
    SlugAudit-owned wire schema.
12. Add a dependency API probe and required serialization feature decision.
13. Add property/boundary tests and an error matrix.
14. Add evidence normalization tracing fields and redaction tests.

## Phase 1 decision

FAIL. The phase is not sufficiently precise to serve as the stable contract for
the database or MCP layers. Correct the master plan before Phase 2 begins.
