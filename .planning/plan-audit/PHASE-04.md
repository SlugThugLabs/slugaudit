# Plan Audit — Phase 4

Status: audited — FAIL

## Verdict

The language-pack direction is correct, but “306 languages” is treated too
close to “306 equally supported intelligence profiles.” Upstream documentation
confirms on-demand caching, `process()` intelligence, low-level `get_parser()`,
aliases, varying query coverage, and ABI variation. The plan must model those
capabilities explicitly.

## Findings

1. Parser availability, download, load, parse diagnostics, and extraction
   completeness are still easy to collapse into one status.
2. On-demand downloads can block a sync, fail offline, or be corrupted. The
   plan lacks cache permissions, checksum verification, atomic cache install,
   and retry policy.
3. The full catalog includes grammars with different query coverage and ABI
   versions. A catalog count alone is not a capability test.
4. `process()` is not an arbitrary Tree-sitter query API. The plan must not
   create a generic query-execution feature by implication.
5. Raw AST fallback can explode storage and response size. It needs budgets,
   node selection, and truncation metadata.
6. Parser reuse is not reconciled with parallel workers. A parser-per-file
   design is wasteful; one shared mutable parser across threads is unsafe.
7. Symbols are identifiers, not complete references or semantic bindings.
8. Diagnostics are recoverable syntax evidence, not necessarily a total parse
   failure. The plan must retain partial structure.
9. Language aliases and case sensitivity are not included in freshness or
   persistence rules.
10. ABI 14/15 compatibility is asserted upstream but not tested in the plan
    against the pinned runtime.

## Required corrections

Add a persisted language capability matrix, cache integrity policy, offline
mode, per-worker parser reuse, ABI representative tests, alias normalization,
and explicit process-vs-raw-parser boundaries. Surface “parser available but
extraction partial” to the AI.

## Testing / observability

Test cold/warm/offline/corrupt cache, a language outside the old eight, ABI 14
and 15 samples, missing query capabilities, malformed source with partial
results, and deterministic reused-parser batches. Log cache event, language,
capability, duration, and outcome; never log source text.
