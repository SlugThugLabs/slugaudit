# Phase 9 golden responses

Captured from the pre-store tools (tool behaviour of 04189b9, equal to 2ad7bc3)
before the paging rewrite. Phase 9.3 must keep every response equal to these
fixtures field for field, apart from the paging fields masked below.

Each fixture is `{ "case", "request", "response" }`. Only `response` is compared.
The comparison test is `src/tools/golden_phase9_tests.rs` (a lib unit test, so it
can use `cfg(test)` items); run it with
`cargo test --lib golden_phase9 --all-features --locked`. To regenerate every
fixture run `tests/golden/phase9/generate.sh`, which sets
`GOLDEN_PHASE9_RECORD=1` for the same tests (the variable is only read).

## Normalised before storing and before comparing
- `revision_id` (any depth; `page.revision_id`) -> `<REVISION_ID>`.
- The absolute temp project path inside any string -> `<PROJECT>` (none appear
  today; the substitution is a guard).
- No durations, timestamps, uuids or counters occur in these responses.

## Paging fields masked at comparison (both sides -> `<PAGING>`)
Masked by key name wherever they occur: `offset`, `next_offset`,
`expected_revision`, `cursor`. In today's responses that is `page.next_offset`
(on `leads`, `query`, `structure`, `finding_read`, `mechanic.first_count`); the
others occur only in requests. Fixtures keep the recorded raw values.
NOT masked: `page.returned`, `total`, `complete`, `page_bytes`, `bytes_used`,
`limited_by`, `structure`'s second cursor `next_start_after_file`, and
`clipped_cells`/`non_finite_cells`. If 9.3 changes one of those by design it must
extend the mask list and record the reason in DEVIATIONS.md.

## NaN
No route yields a REAL NaN at the result in this build: the bundled SQLite has no
math functions (`sqrt`, `acos`, `log` are "no such function"), arithmetic that
would give NaN (`1e308*10*0`, `inf - inf`) comes back as NULL, and a `cfg(test)`
scalar function `golden_nan()` (in `src/store/sql_functions.rs`) that returns
`f64::NAN` also reaches the result as NULL, because SQLite turns a NaN into NULL
when a result is set. `query_nan_expressions` therefore records three NULL cells
with an empty `non_finite_cells`. The `"NaN"` branch in `query_value.rs` is
unreachable today.

## Cases -> files
| Case | Fixture |
|---|---|
| leads counts only | `leads_counts_only.json` |
| leads rows page 1 / page 2 (limit 5, offset contract) | `leads_rows_page1.json`, `leads_rows_page2.json` |
| leads compact | `leads_compact.json` |
| leads custom mechanic | `leads_custom_mechanic.json` |
| query plain rows | `query_plain_rows.json` |
| query clipped cells (`continue_with`) | `query_clipped_cells.json` |
| query +Inf / -Inf (`non_finite_cells`) | `query_non_finite_inf.json` |
| query NaN attempts (all NULL, see above) | `query_nan_expressions.json` |
| query text with NUL | `query_text_with_nul.json` |
| query blob | `query_blob.json` |
| query numeric-looking TEXT | `query_numeric_text.json` |
| query NULL | `query_null.json` |
| query i64 extremes | `query_i64_extremes.json` |
| query multi-page (page 1 / page 2) | `query_multi_page1.json`, `query_multi_page2.json` |
| structure single file | `structure_single_file.json` |
| structure project-wide | `structure_project_wide.json` |
| finding_read list / offset 1 | `finding_read_list.json`, `finding_read_offset1.json` |
| mechanic add / list | `mechanic_add.json`, `mechanic_list.json` |
