# Plan Audit — Phase 3

Status: audited — FAIL

## Verdict

The synchronization sequence is sensible but incomplete. It does not prove a
stable snapshot when files change during discovery, hashing, parsing, or
publication, and its ignore/symlink rules are not precise enough to guarantee
complete evidence without escaping the project root.

## Findings

1. `hash -> parse -> publish` is not a stable snapshot if a file changes after
   hashing. The plan tests a change during hashing only, not change during
   parsing or before commit. Require a second hash verification before publish
   and bounded retry behavior.
2. “Respect repository ignore rules where supported” is vague. Define which
   ignore files and precedence rules apply and persist the discovery policy in
   the manifest.
3. “Binary files are classified” does not define whether metadata, hash, and
   path are stored or whether content is excluded from search.
4. Symlink handling does not define in-root symlink behavior, loops, broken
   links, or canonicalization races.
5. Activation is named but there is no atomic enable/disable lifecycle or rule
   for a database left after marker removal.
6. Manifest equality does not define ordering, parser catalog capability hash,
   ignore-policy version, or extraction version inputs.
7. Atomic revision publication has no read-side protocol: a reader can begin
   before a revision swap and finish after it unless all queries bind one
   revision snapshot.
8. Two concurrent syncs have no lock ownership, timeout, loser behavior, or
   stale-lock policy in this phase.
9. Deletion during discovery is not distinguished from permission/error
   omission. Treating an unreadable file as deleted can destroy evidence.
10. The plan does not define whether an empty repository is valid or how a
    project with only binary/ignored files is represented.

## Testing gaps

Add tests for changes during parse and pre-commit rehash, ignore precedence,
in-root/broken/cyclic symlinks, unreadable files, reader revision pinning,
concurrent lock timeout, empty projects, and manifest policy changes.

## Required corrections

Add a `DiscoveryOutcome` distinction between indexed, excluded, unreadable,
binary, and vanished. Add a stable snapshot protocol with final rehash. Add a
per-project lock contract and make every query bind a verified revision handle.

## Complexity / operations

No implementation functions exist yet. Require complexity reporting for the
discovery walker and publish state machine. Log discovery counts, retry counts,
lock wait, parse/publish duration, and omission reasons without source content.
