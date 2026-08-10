# Evidence contract — multilang fixture

Versioned evidence contract for the Phase 12 acceptance fixture
(`tests/fixtures/multilang/`). The machine-readable golden manifest is
`MANIFEST.json` (contract version `1`); `tests/fixture_contract.rs`
asserts that a real `sync::publish` over a copy of this fixture produces
exactly that manifest.

## What the fixture exercises

| File(s) | Purpose |
|---|---|
| `rust/` (crate with `crate::util` import) | Rust parsing; workspace-aware `crate::` resolution (High confidence) |
| `python/package/` (relative imports, circular pair) | Python parsing; relative-import resolution; circular imports resolve in both directions without looping |
| `python/package/broken.py` | Intentionally malformed source: must be `SyntaxErrors` with ≥ 1 `Diagnostic` evidence, never a silent clean parse |
| `typescript/` | TypeScript parsing; relative import resolution; external `fs` import recorded as `External` |
| `javascript/` (circular pair) | **The language outside the old Python eight-language set**; parsing + resolution proven for it |
| `go/`, `ruby/` | Parsed (evidence extracted) but import resolution is *not* modeled for these languages: their imports are honestly `Unresolved` (Go) or not captured as imports (Ruby) — never faked as resolved |
| `config/`, `docs/`, `scripts/`, `Makefile`, `Cargo.toml` | Non-source files: parsed as their detected languages (json/yaml/markdown/bash/toml) or honestly `NotAttempted` when no grammar applies (`.gitignore`, `Makefile`) |
| `assets/logo.bin` | Binary file (NUL byte): classified `binary`, no content, no language, no evidence |
| `.gitignore` | Hidden file included by discovery (`hidden(false)`); no grammar → `NotAttempted` |

## Contracted invariants (asserted by `tests/fixture_contract.rs`)

1. **Exact file set** — the stored `files.path` set equals `file_set`.
2. **Per-file records** — `file_kind`, `language`, `parse_outcome`,
   `extraction_completeness`, and per-kind `evidence_counts` match exactly.
3. **Malformed source is visible** — `broken.py` is `SyntaxErrors` with
   exactly one `Diagnostic` evidence item.
4. **Circular imports resolve finitely** — both directions of the Python
   and JavaScript circular pairs appear as `Resolved` edges; nothing loops.
5. **Unsupported languages are honest** — Go/Ruby imports are never
   `Resolved`; the manifest records `unsupported_language_unresolved_count`
   (currently 4, all from Go) exactly as `report` computes it.
6. **Binary files are inert** — `binary` file_kind, no content, no
   evidence.
7. **Searchability** — every indexed file's `content` is stored
   (`indexed_files_with_content == indexed`), so every non-binary file is
   retrievable through `query`.
8. **Exact totals** — file counts, `evidence_by_kind`, and
   `diagnostic_count` match.

## Pinning

- `parser_pack_version` must equal `parse::PACK_VERSION` (the exact
  `=1.13.7` pin). A pack bump therefore fails the contract until the
  manifest is deliberately regenerated.
- `contract_version` bumps independently when the manifest's *meaning*
  changes (new fields, stricter expectations) — it must match
  `CONTRACT_VERSION` in the test.
- Evidence counts are deterministic because the pack is exactly pinned;
  `tests/fixture_contract.rs` has passed 5 consecutive runs without
  changes.

## Regeneration procedure

Only for a deliberate parser-pack version bump or a fixture edit that
intentionally changes expected evidence:

```bash
SLUGAUDIT_REGEN_MANIFEST=1 cargo test --test fixture_contract -- --nocapture
```

The regen run prints the full raw `dependency_edges` table to stderr.
**Review the regenerated manifest by hand** (malformed file still flagged,
circular pairs still both directions, binary still inert, unsupported
languages still unresolved) before committing. Never commit a regenerated
manifest without that review — the golden-manifest discipline is the point
of this contract (plan-audit PHASE-12).

## Fixture hygiene

- Tests never mutate the checked-in fixture; they publish a temp-dir copy
  (excluding `MANIFEST.json` and this document).
- The copy's database lives in `.planning/slugaudit/project.db` — the
  production layout — which discovery excludes by construction.
- Per-file `content_hash` pins the fixture's source bytes: any edit to a
  fixture file changes the hash and fails the contract until the manifest
  is deliberately regenerated.
- Gitignore divergence to be aware of: the temp copy has no parent `.git`
  repository, so the `ignore` crate does **not** apply the fixture's own
  `.gitignore` rules there, while the checked-in location would. A future
  fixture file matching the fixture's `.gitignore` patterns (e.g.
  `node_modules/`) would be excluded in place but indexed in the copy —
  keep fixture `.gitignore` rules in sync with what the contract expects
  to index.
