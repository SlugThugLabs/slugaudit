# Sync `publish.rs` Split Plan

## Trigger

This plan activates when `src/sync/publish.rs` reaches **250 code lines**
(the source-size gate allows 200–300 with an exception comment; 250 is the
early-warning threshold that gives us room to act before the 300-line hard
limit).

Check with: `bash tools/check_source_limits.sh`

## Current State

`publish.rs` (192 lines) orchestrates the full publish flow:

```
publish()              -- retry loop
  └── try_publish()    -- discover, sample, diff, revalidate, write
        ├── current_revision()
        ├── discovery::discover()
        ├── sample_all()
        ├── race_hook::fire()
        ├── diff_against_stored()
        ├── build_upserts_and_deletions()
        ├── revalidate_unchanged_since_sample()
        └── revision::publish_revision()
```

The retry loop (`publish`) and the single-attempt logic (`try_publish`) are
currently in the same file. The retry logic is the natural growth point —
each new retryable error variant, each new CAS concern, adds branches to
`publish()`.

## The Split

Extract the retry/CAS loop into `src/sync/publish_cas.rs`. After the split:

```
src/sync/
  publish.rs          -- thin wrapper: calls publish_cas::publish_with_retry()
  publish_cas.rs      -- retry loop, CAS semantics, outcome logging
  publish_diff.rs     -- diff building (already separate)
  publish_log.rs      -- outcome logging (already separate)
```

### `publish_cas.rs` owns:

- `MAX_CAS_RETRIES` constant
- `is_retryable()` predicate
- The `publish()` retry loop (currently in `publish.rs`)
- `try_publish()` body (the single-attempt logic)
- `current_revision()` helper
- `sample_all()` helper (or move to `sample.rs`)
- `revalidate_unchanged_since_sample()` (or move to `revision.rs`)

### `publish.rs` becomes:

```rust
pub fn publish(connection, root, parser_pack_version) -> Result<PublishReport, PublishError> {
    publish_cas::publish_with_retry(connection, root, parser_pack_version)
}
```

Plus the `PublishError` and `PublishReport` type definitions (which stay
public for callers).

## Steps

1. Create `src/sync/publish_cas.rs`
2. Move `MAX_CAS_RETRIES`, `is_retryable`, `current_revision`,
   `sample_all`, `revalidate_unchanged_since_sample`, `try_publish`,
   and the `publish` retry loop into it
3. Rename `publish` to `publish_with_retry` in the new module
4. Reduce `publish.rs` to the thin wrapper + public types
5. Update `sync/mod.rs` to add `mod publish_cas`
6. Run full CI gate set

## Alternative: Don't Split

If the growth is in `try_publish`'s orchestration (more steps added) rather
than the retry loop, the right split is different:

- Extract `revalidate_unchanged_since_sample` into `src/sync/revalidate.rs`
- Extract `sample_all` into `src/sync/sample.rs` (it's already called from
  `sample.rs::to_file_record`'s vicinity but `sample_all` lives in
  `publish.rs`)

This keeps `publish.rs` as the orchestrator but moves heavy steps out.

Decision at time of split: follow the growth. If it's retry logic, use the
`publish_cas` split above. If it's orchestration steps, extract steps into
their own modules.

## Test Impact

Minimal. The `#[cfg(test)] mod tests` in `publish.rs` moves with the code
into `publish_cas.rs` (or stays in `publish.rs` testing the thin wrapper).
The race tests (`publish_race_tests.rs`) call `publish()` which still exists
as the public entry point — no changes needed there.
